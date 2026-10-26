use emmylua_code_analysis::{read_file_with_encoding, uri_to_file_path};
use lsp_types::{
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DidSaveTextDocumentParams,
};

use crate::context::{ServerContextSnapshot, UpdateEvent};

pub async fn on_did_open_text_document(
    context: ServerContextSnapshot,
    params: DidOpenTextDocumentParams,
) -> Option<()> {
    let _ = context.update_tx().send(UpdateEvent::DidOpen(params));
    Some(())
}

pub async fn process_did_open_text_document(
    context: ServerContextSnapshot,
    params: DidOpenTextDocumentParams,
) -> Option<()> {
    let version = params.text_document.version;
    let uri = params.text_document.uri;
    let text = params.text_document.text;

    // Check if file should be filtered before acquiring locks
    // Follow lock order: workspace_manager (read) -> analysis (write)
    let should_process = match context.analysis().try_with_snapshot(|analysis| {
        let old_file_id = analysis.get_file_id(&uri);
        if old_file_id.is_some() {
            Some(true)
        } else {
            None
        }
    }) {
        Some(true) => true,
        _ => {
            let workspace_manager = context.workspace_manager().lock().await;
            workspace_manager.is_workspace_file(&uri)
        }
    };

    {
        let mut workspace = context.workspace_manager().lock().await;
        workspace.sync_open_file(uri.clone(), text.clone());
    }

    if !should_process {
        return None;
    }

    // Update file and get diagnostic settings
    let (file_id, interval) = context
        .analysis()
        .update(|analysis| {
            let file_id = analysis.update_file_by_uri(&uri, Some(text));
            let interval = analysis
                .get_emmyrc()
                .diagnostics
                .diagnostic_interval
                .unwrap_or(500);
            (file_id, interval)
        })
        .await;
    if file_id.is_some() {
        let mut workspace = context.workspace_manager().lock().await;
        workspace.open_file_versions.insert(uri.clone(), version);
        workspace.update_workspace_version(crate::context::WorkspaceDiagnosticLevel::Slow, true);
    }
    let supports_pull = context.lsp_features().supports_pull_diagnostic();

    // Schedule diagnostic task without holding any locks
    if !supports_pull {
        if let Some(file_id) = file_id {
            context
                .file_diagnostic()
                .add_diagnostic_task(file_id, interval)
                .await;
        }
    }

    Some(())
}

pub async fn on_did_save_text_document(
    context: ServerContextSnapshot,
    params: DidSaveTextDocumentParams,
) -> Option<()> {
    let _ = context.update_tx().send(UpdateEvent::DidSave(params));
    Some(())
}

pub async fn process_did_save_text_document(
    context: ServerContextSnapshot,
    _: DidSaveTextDocumentParams,
) -> Option<()> {
    // The synchronized document already contains the saved text. Do not cancel
    // the client's pending pull; it will observe this generation and rerun.
    context.invalidate_workspace_diagnostics().await;
    Some(())
}

pub async fn on_did_change_text_document(
    context: ServerContextSnapshot,
    params: DidChangeTextDocumentParams,
) -> Option<()> {
    // Only enqueue; don't block the notification loop. Worker processes serially to preserve order.
    let _ = context.update_tx().send(UpdateEvent::DidChange(params));
    Some(())
}

pub async fn process_did_change_text_document(
    context: ServerContextSnapshot,
    params: DidChangeTextDocumentParams,
) -> Option<()> {
    let version = params.text_document.version;
    let uri = params.text_document.uri;
    let text = params.content_changes.first()?.text.clone();

    // Check if file should be filtered before acquiring locks
    // Follow lock order: workspace_manager (read) -> analysis (write)
    let should_process = match context.analysis().try_with_snapshot(|analysis| {
        let old_file_id = analysis.get_file_id(&uri);
        if old_file_id.is_some() {
            Some(true)
        } else {
            None
        }
    }) {
        Some(true) => true,
        _ => {
            let workspace_manager = context.workspace_manager().lock().await;
            workspace_manager.is_workspace_file(&uri)
        }
    };

    {
        let mut workspace = context.workspace_manager().lock().await;
        workspace.sync_open_file(uri.clone(), text.clone());
    }

    if !should_process {
        return None;
    }

    // Update file and get settings
    let (file_id, interval) = context
        .analysis()
        .update(|analysis| {
            let file_id = analysis.update_file_by_uri(&uri, Some(text));
            let emmyrc = analysis.get_emmyrc();
            (
                file_id,
                emmyrc.diagnostics.diagnostic_interval.unwrap_or(500),
            )
        })
        .await;

    if file_id.is_some() {
        let mut workspace = context.workspace_manager().lock().await;
        workspace.open_file_versions.insert(uri.clone(), version);
        workspace.update_workspace_version(crate::context::WorkspaceDiagnosticLevel::Slow, true);
    }
    let supports_pull = context.lsp_features().supports_pull_diagnostic();
    // Schedule diagnostic task
    if !supports_pull {
        if let Some(file_id) = file_id {
            context
                .file_diagnostic()
                .add_diagnostic_task(file_id, interval)
                .await;
        }
    }

    Some(())
}

pub async fn on_did_close_document(
    context: ServerContextSnapshot,
    params: DidCloseTextDocumentParams,
) -> Option<()> {
    let _ = context.update_tx().send(UpdateEvent::DidClose(params));
    Some(())
}

pub async fn process_did_close_document(
    context: ServerContextSnapshot,
    params: DidCloseTextDocumentParams,
) -> Option<()> {
    let uri = &params.text_document.uri;
    let mut workspace = context.workspace_manager().lock().await;
    workspace.close_open_file(&params.text_document.uri);
    drop(workspace);
    let lsp_features = context.lsp_features();

    // If the closed file no longer exists, remove it.
    if let Some(file_path) = uri_to_file_path(uri)
        && !file_path.exists()
    {
        let removed = context
            .analysis()
            .update(|analysis| {
                let tracked = analysis.get_file_id(uri).is_some();
                analysis.remove_file_by_uri(uri);
                tracked
            })
            .await;
        if removed {
            context.invalidate_workspace_diagnostics().await;
        }

        if !lsp_features.supports_pull_diagnostic() {
            context
                .file_diagnostic()
                .clear_push_file_diagnostics(uri.clone());
        }

        return Some(());
    }

    // Non-workspace file (not registered in semantic) -> remove.
    let file_exists = context
        .analysis()
        .try_with_snapshot(|analysis| {
            let file_id = analysis.get_file_id(uri)?;
            Some(analysis.db.get_file_text(file_id).is_some())
        })
        .unwrap_or(false);
    if !file_exists {
        let removed = context
            .analysis()
            .update(|analysis| {
                let tracked = analysis.get_file_id(uri).is_some();
                analysis.remove_file_by_uri(uri);
                tracked
            })
            .await;
        if removed {
            context.invalidate_workspace_diagnostics().await;
        }

        if !lsp_features.supports_pull_diagnostic() {
            context
                .file_diagnostic()
                .clear_push_file_diagnostics(uri.clone());
        }
    }

    // Closing hands authority back to disk. Changes observed while the buffer
    // was open were deliberately ignored by the file watcher.
    if file_exists
        && let Some(path) = uri_to_file_path(uri)
        && let Some(config) = context
            .analysis()
            .with_snapshot(|analysis| analysis.get_emmyrc())
        && let Some(text) = read_file_with_encoding(&path, &config.workspace.encoding)
    {
        let changed = context
            .analysis()
            .update(|analysis| {
                let file_id = analysis.get_file_id(uri)?;
                if analysis.db.get_file_text(file_id) == Some(text.as_str()) {
                    return None;
                }
                analysis.update_file_by_uri(uri, Some(text))
            })
            .await;
        if let Some(file_id) = changed {
            context.invalidate_workspace_diagnostics().await;
            if !lsp_features.supports_pull_diagnostic() {
                context
                    .file_diagnostic()
                    .add_diagnostic_task(
                        file_id,
                        config.diagnostics.diagnostic_interval.unwrap_or(500),
                    )
                    .await;
            }
        }
    }

    Some(())
}

#[cfg(test)]
mod close_tests {
    use super::*;
    use crate::context::ServerContext;
    use lsp_server::Connection;
    use lsp_types::{ClientCapabilities, TextDocumentIdentifier, Uri};
    use std::sync::Arc;

    #[tokio::test]
    async fn closing_document_reloads_disk_and_preserves_untracked_files() {
        let directory = std::env::temp_dir().join(format!(
            "emmylua-close-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let file = directory.join("dependency.lua");
        std::fs::write(&file, "local disk = 1").unwrap();
        let uri: Uri = emmylua_code_analysis::file_path_to_uri(&file).unwrap();
        let (server, _client) = Connection::memory();
        let context = ServerContext::new(Arc::new(server), ClientCapabilities::default());
        let snapshot = context.snapshot();
        snapshot
            .analysis()
            .update(|analysis| {
                analysis.update_file_by_uri(&uri, Some("local buffer = 2".to_string()));
            })
            .await;
        snapshot
            .workspace_manager()
            .lock()
            .await
            .sync_open_file(uri.clone(), "local buffer = 2".into());
        process_did_close_document(
            snapshot.clone(),
            DidCloseTextDocumentParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
            },
        )
        .await;
        assert!(!snapshot.workspace_manager().lock().await.is_open_file(&uri));
        snapshot.analysis().with_snapshot(|analysis| {
            let file_id = analysis.get_file_id(&uri).unwrap();
            assert_eq!(
                analysis.db.get_file_text(file_id).unwrap().to_string(),
                "local disk = 1"
            );
        });
        // Files which were never part of this server must not be imported by closing them.
        let other = directory.join("untracked.lua");
        std::fs::write(&other, "local other = 1").unwrap();
        let other_uri: Uri = emmylua_code_analysis::file_path_to_uri(&other).unwrap();
        process_did_close_document(
            snapshot.clone(),
            DidCloseTextDocumentParams {
                text_document: TextDocumentIdentifier {
                    uri: other_uri.clone(),
                },
            },
        )
        .await;
        assert!(
            snapshot
                .analysis()
                .with_snapshot(|analysis| analysis.get_file_id(&other_uri))
                .flatten()
                .is_none()
        );
        // A document deleted on disk still disappears from analysis when closed.
        std::fs::remove_file(file).unwrap();
        process_did_close_document(
            snapshot.clone(),
            DidCloseTextDocumentParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
            },
        )
        .await;
        assert!(
            snapshot
                .analysis()
                .with_snapshot(|analysis| analysis
                    .db
                    .get_file_text(analysis.get_file_id(&uri)?)
                    .map(str::to_owned))
                .flatten()
                .is_none()
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
