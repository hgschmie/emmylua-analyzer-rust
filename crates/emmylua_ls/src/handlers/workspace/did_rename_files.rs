use std::{collections::HashMap, path::Path, str::FromStr};

use emmylua_code_analysis::{
    EmmyLuaAnalysis, SemanticDatabase, SemanticModel, file_path_to_uri, read_file_with_encoding,
    uri_to_file_path,
};
use emmylua_parser::{LuaAstNode, LuaAstToken, LuaCallExpr, LuaLiteralToken};
use lsp_types::{
    ApplyWorkspaceEditParams, FileRename, MessageActionItem, MessageType, RenameFilesParams,
    ShowMessageRequestParams, TextEdit, Uri, WorkspaceEdit,
};
use tokio::{
    sync::{Mutex, mpsc, oneshot},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
use walkdir::WalkDir;

use crate::{
    context::{ServerContextSnapshot, UpdateEvent},
    handlers::ClientConfig,
};

pub async fn on_did_rename_files_handler(
    context: ServerContextSnapshot,
    params: RenameFilesParams,
) -> Option<()> {
    let _ = context
        .update_tx()
        .send(UpdateEvent::DidRenameFiles(params));
    Some(())
}

pub async fn process_did_rename_files_handler(
    context: ServerContextSnapshot,
    params: RenameFilesParams,
) -> Option<()> {
    // Collect rename info using a snapshot; release it immediately after collection to avoid later update() waiting on its own snapshot.
    let all_renames = context
        .analysis()
        .try_with_snapshot(|analysis| {
            let db = &analysis.db;
            let mut all_renames: Vec<RenameInfo> = vec![];

            for file_rename in params.files {
                let FileRename { old_uri, new_uri } = file_rename;

                let old_uri = Uri::from_str(&old_uri).ok()?;
                let new_uri = Uri::from_str(&new_uri).ok()?;

                let old_path = uri_to_file_path(&old_uri)?;
                let new_path = uri_to_file_path(&new_uri)?;

                let rename_info = collect_rename_info(&old_uri, &new_uri, &db);
                if let Some(rename_info) = rename_info {
                    all_renames.push(rename_info.clone());
                } else if let Some(collected_renames) =
                    collect_directory_lua_files(&old_path, &new_path, &db)
                {
                    all_renames.extend(collected_renames);
                }
            }

            Some(all_renames)
        })
        .unwrap_or_default();

    // If there are renamed files, prompt the user whether to update require paths.
    if !all_renames.is_empty() {
        let encoding = context
            .analysis()
            .with_snapshot(|analysis| analysis.get_emmyrc().workspace.encoding.clone())
            .unwrap_or_default();
        context
            .analysis()
            .update(|analysis| {
                for rename in all_renames.iter() {
                    analysis.remove_file_by_uri(&rename.old_uri);
                    if let Some(new_path) = uri_to_file_path(&rename.new_uri)
                        && let Some(text) = read_file_with_encoding(&new_path, &encoding)
                    {
                        analysis.update_file_by_uri(&rename.new_uri, Some(text));
                    }
                }
            })
            .await;

        // Queue one client interaction per notification, after its analysis
        // mutations. User interaction must not occupy the update worker.
        context
            .rename_client_queue()
            .enqueue(context.clone(), all_renames);
    }

    Some(())
}

/// Serializes rename prompts and edits without blocking document updates.
pub(crate) struct RenameClientQueue {
    sender: mpsc::UnboundedSender<(ServerContextSnapshot, Vec<RenameInfo>)>,
    cancel_token: CancellationToken,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl RenameClientQueue {
    pub(crate) fn new() -> Self {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let cancel_token = CancellationToken::new();
        let worker_token = cancel_token.clone();
        let worker = tokio::spawn(async move {
            loop {
                let job = tokio::select! {
                    biased;
                    _ = worker_token.cancelled() => break,
                    job = receiver.recv() => job,
                };
                let Some((context, renames)) = job else {
                    break;
                };
                update_require_paths(context, renames, worker_token.clone()).await;
            }
        });
        Self {
            sender,
            cancel_token,
            worker: Mutex::new(Some(worker)),
        }
    }

    fn enqueue(&self, context: ServerContextSnapshot, renames: Vec<RenameInfo>) {
        if !self.cancel_token.is_cancelled() && self.sender.send((context, renames)).is_err() {
            log::error!("Rename client worker unavailable");
        }
    }

    pub(crate) async fn close(&self) {
        self.cancel_token.cancel();
        // Keep the guard until the worker finishes so repeated closes also
        // wait for client response registrations to be cleaned up.
        let mut worker = self.worker.lock().await;
        if let Some(worker) = worker.take()
            && let Err(error) = worker.await
        {
            log::error!("Rename client worker stopped unexpectedly: {error}");
        }
    }
}

async fn wait_for_rename_updates(
    context: &ServerContextSnapshot,
    cancel_token: &CancellationToken,
) -> bool {
    if cancel_token.is_cancelled() {
        return false;
    }
    let (ready, updates_applied) = oneshot::channel();
    let _ = context.update_tx().send(UpdateEvent::Barrier(ready));
    tokio::select! {
        biased;
        _ = cancel_token.cancelled() => false,
        result = updates_applied => {
            if let Err(error) = result {
                log::error!("Update worker unavailable for rename edits: {error}");
                false
            } else {
                true
            }
        }
    }
}

async fn update_require_paths(
    context: ServerContextSnapshot,
    renames: Vec<RenameInfo>,
    cancel_token: CancellationToken,
) -> Option<()> {
    // Earlier batches may have changed the references while this one waited.
    if !wait_for_rename_updates(&context, &cancel_token).await {
        return None;
    }
    let has_changes = context.analysis().try_with_snapshot(|analysis| {
        Some(!try_modify_require_path(analysis, &renames)?.is_empty())
    })?;
    if !has_changes || cancel_token.is_cancelled() {
        return None;
    }
    let client = context.client();
    let selected_action = client
        .show_message_request(
            ShowMessageRequestParams {
                typ: MessageType::INFO,
                message: t!("Do you want to modify the require path?").to_string(),
                actions: Some(vec![MessageActionItem {
                    title: t!("Modify").to_string(),
                    properties: HashMap::new(),
                }]),
            },
            cancel_token.clone(),
        )
        .await?;
    if selected_action.title != t!("Modify")
        || !wait_for_rename_updates(&context, &cancel_token).await
    {
        return None;
    }

    // Recompute ranges from the live analysis after confirmation and preceding
    // updates; never retain TextEdits across the user's potentially long wait.
    // The existing unversioned protocol still cannot guard client edits in flight.
    let changes = context
        .analysis()
        .try_with_snapshot(|analysis| try_modify_require_path(analysis, &renames))?;
    if changes.is_empty() || cancel_token.is_cancelled() {
        return None;
    }
    let response = client
        .apply_edit(
            ApplyWorkspaceEditParams {
                edit: WorkspaceEdit {
                    changes: Some(changes),
                    document_changes: None,
                    change_annotations: None,
                },
                label: None,
            },
            cancel_token,
        )
        .await?;
    if !response.applied {
        log::warn!(
            "Client did not apply rename edits: {}",
            response
                .failure_reason
                .as_deref()
                .unwrap_or("no reason supplied")
        );
    }
    Some(())
}

#[derive(Debug, PartialEq, Eq, Clone)]
struct RenameInfo {
    old_uri: Uri,
    new_uri: Uri,
    old_module_path: String,
    new_module_path: String,
}

fn collect_rename_info(old_uri: &Uri, new_uri: &Uri, db: &SemanticDatabase) -> Option<RenameInfo> {
    let old_module_path = db
        .module_name_from_path(&uri_to_file_path(old_uri)?)?
        .replace(['\\', '/'], ".");
    let new_module_path = db
        .module_name_from_path(&uri_to_file_path(new_uri)?)?
        .replace(['\\', '/'], ".");

    Some(RenameInfo {
        old_uri: old_uri.clone(),
        new_uri: new_uri.clone(),
        old_module_path,
        new_module_path,
    })
}

/// Collect all Lua files affected by a directory rename.
fn collect_directory_lua_files(
    old_path: &Path,
    new_path: &Path,
    db: &SemanticDatabase,
) -> Option<Vec<RenameInfo>> {
    // Check that the new path is a directory (the old path no longer exists).
    if !new_path.is_dir() {
        return None;
    }

    let mut renames = vec![];

    // Walk all Lua files under the new directory.
    for entry in WalkDir::new(new_path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let new_file_path = entry.path();

        // Compute the path relative to the new directory.
        if let Ok(relative_path) = new_file_path.strip_prefix(new_path) {
            // Infer the corresponding old file path from the directory rename.
            let old_file_path = old_path.join(relative_path);

            // Convert to URI.
            if let (Some(old_file_uri), Some(new_file_uri)) = (
                file_path_to_uri(&old_file_path),
                file_path_to_uri(&new_file_path.to_path_buf()),
            ) {
                let rename_info = collect_rename_info(&old_file_uri, &new_file_uri, db);
                if let Some(rename_info) = rename_info {
                    renames.push(rename_info);
                }
            }
        }
    }

    if renames.is_empty() {
        None
    } else {
        Some(renames)
    }
}

#[allow(unused)]
/// Check whether a file path is a Lua file.
fn is_lua_file(file_path: &Path, client_config: &ClientConfig) -> bool {
    let file_name = file_path.to_string_lossy();

    if file_name.ends_with(".lua") {
        return true;
    }

    // Check client-configured extensions.
    for extension in &client_config.extensions {
        if file_name.ends_with(extension) {
            return true;
        }
    }

    false
}

fn try_modify_require_path(
    analysis: &EmmyLuaAnalysis,
    renames: &[RenameInfo],
) -> Option<HashMap<Uri, Vec<TextEdit>>> {
    #[allow(clippy::mutable_key_type)]
    let mut changes: HashMap<Uri, Vec<TextEdit>> = HashMap::new();
    let db = &analysis.db;
    for file_id in db.file_ids() {
        let model = SemanticModel::new(db, file_id);
        let Some(chunk) = model.chunk() else {
            continue;
        };
        for call_expr in chunk.descendants::<LuaCallExpr>() {
            if call_expr.is_require() {
                try_convert(analysis, &db, file_id, call_expr, renames, &mut changes);
            }
        }
    }
    Some(changes)
}

#[allow(clippy::mutable_key_type)]
fn try_convert(
    analysis: &EmmyLuaAnalysis,
    db: &SemanticDatabase,
    file_id: emmylua_code_analysis::FileId,
    call_expr: LuaCallExpr,
    renames: &[RenameInfo],
    changes: &mut HashMap<Uri, Vec<TextEdit>>,
) -> Option<()> {
    let args_list = call_expr.get_args_list()?;
    let arg_expr = args_list.get_args().next()?;
    // String literal argument → module name.
    let literal = arg_expr
        .syntax()
        .descendants_with_tokens()
        .filter_map(|it| it.into_token())
        .find_map(LuaLiteralToken::cast)?;
    let name = match literal {
        LuaLiteralToken::String(string) => string.get_value(),
        _ => return None,
    };

    let emmyrc = analysis.get_emmyrc();
    let separator = &emmyrc.completion.auto_require_separator;
    let strict_require_path = emmyrc.strict.require_path;
    // Convert to standard import syntax.
    let normalized_path = name.replace(separator, ".");

    for rename in renames {
        let is_matched = if strict_require_path {
            rename.old_module_path == normalized_path
        } else {
            rename.old_module_path.ends_with(&normalized_path)
        };

        if is_matched {
            let document = db.document(file_id)?;
            let range = arg_expr.syntax().text_range();
            let lsp_range = document.to_lsp_range(range)?;
            let current_uri = document.get_uri()?;

            let full_module_path = match separator.as_str() {
                "." | "" => rename.new_module_path.clone(),
                _ => rename.new_module_path.replace(".", separator),
            };

            changes.entry(current_uri).or_default().push(TextEdit {
                range: lsp_range,
                new_text: format!("'{}'", full_module_path),
            });

            return Some(());
        }
    }

    Some(())
}
