use lsp_types::{
    FullDocumentDiagnosticReport, WorkspaceDiagnosticParams, WorkspaceDiagnosticReport,
    WorkspaceFullDocumentDiagnosticReport,
};
use tokio_util::sync::CancellationToken;

use std::{collections::HashMap, time::Duration};

use crate::context::{
    CancelSource, RequestOutcome, ServerContextSnapshot, WorkspaceDiagnosticLevel,
};

pub async fn on_pull_workspace_diagnostic(
    context: ServerContextSnapshot,
    _: WorkspaceDiagnosticParams,
    token: CancellationToken,
) -> RequestOutcome<WorkspaceDiagnosticReport> {
    let requests = context.request_manager();
    requests.begin("workspace_diagnostic", token.clone()).await;
    let lock = requests.lock_for("workspace_diagnostic").await;
    let _guard = tokio::select! {
        _ = token.cancelled() => return RequestOutcome::Cancelled(CancelSource::Client),
        guard = lock.lock() => guard,
    };
    let first = !context.file_diagnostic().has_workspace_diagnostic_done();
    let last_returned_version = context.file_diagnostic().get_last_workspace_version().await;

    loop {
        if token.is_cancelled() {
            return RequestOutcome::Cancelled(CancelSource::Client);
        }

        let (version, status, document_versions) = {
            let workspace = context.workspace_manager().lock().await;
            (
                workspace.get_workspace_version(),
                workspace.get_workspace_diagnostic_level(),
                workspace.open_file_versions.clone(),
            )
        };

        if first
            || last_returned_version != Some(version)
            || status != WorkspaceDiagnosticLevel::None
        {
            // Fast is a parallel initial pass; subsequent passes yield between
            // files so interactive document queries and updates retain priority.
            let result = if first {
                context
                    .file_diagnostic()
                    .pull_workspace_diagnostics_fast(token.clone())
                    .await
            } else {
                context
                    .file_diagnostic()
                    .pull_workspace_diagnostics_slow(token.clone())
                    .await
            };
            if token.is_cancelled() {
                return RequestOutcome::Cancelled(CancelSource::Client);
            }
            if let Some(mut diagnostics) = result {
                let workspace = context.workspace_manager().lock().await;
                if workspace.get_workspace_version() == version {
                    // Only acknowledge a complete, current pass. An edit during
                    // diagnosis must keep the workspace dirty for a retry.
                    context
                        .file_diagnostic()
                        .include_removed_workspace_files(&mut diagnostics)
                        .await;
                    context
                        .file_diagnostic()
                        .set_last_workspace_version(version)
                        .await;
                    context.file_diagnostic().mark_workspace_diagnostic_done();
                    workspace.update_workspace_version(WorkspaceDiagnosticLevel::None, false);
                    return RequestOutcome::Ready(build_workspace_report(
                        diagnostics,
                        &document_versions,
                    ));
                }
            }
        }

        // Coalesce edits and keep an idle pull pending. Cancellation must wake
        // this wait immediately, including cancellation during server shutdown.
        let interval = context
            .analysis()
            .with_snapshot(|analysis| {
                analysis
                    .get_emmyrc()
                    .diagnostics
                    .diagnostic_interval
                    .unwrap_or(500)
            })
            .unwrap_or(500)
            .max(100);
        tokio::select! {
            _ = token.cancelled() => return RequestOutcome::Cancelled(CancelSource::Client),
            _ = tokio::time::sleep(Duration::from_millis(interval)) => {},
        }
    }
}

fn build_workspace_report(
    file_diagnostics: Vec<(lsp_types::Uri, Vec<lsp_types::Diagnostic>)>,
    document_versions: &HashMap<lsp_types::Uri, i32>,
) -> WorkspaceDiagnosticReport {
    WorkspaceDiagnosticReport {
        items: file_diagnostics
            .into_iter()
            .map(|(uri, diagnostics)| {
                WorkspaceFullDocumentDiagnosticReport {
                    // The workspace generation is not an LSP document version.
                    version: document_versions.get(&uri).copied().map(i64::from),
                    uri,
                    full_document_diagnostic_report: FullDocumentDiagnosticReport {
                        items: diagnostics,
                        result_id: None,
                    },
                }
                .into()
            })
            .collect(),
    }
}
