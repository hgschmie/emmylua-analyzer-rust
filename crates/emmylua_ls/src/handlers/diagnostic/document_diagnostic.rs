use lsp_types::{
    DocumentDiagnosticParams, DocumentDiagnosticReport, DocumentDiagnosticReportResult,
    FullDocumentDiagnosticReport, RelatedFullDocumentDiagnosticReport,
};
use tokio_util::sync::CancellationToken;

use crate::context::{CancelSource, RequestOutcome, ServerContextSnapshot, analysis_query};

pub async fn on_pull_document_diagnostic(
    context: ServerContextSnapshot,
    params: DocumentDiagnosticParams,
    token: CancellationToken,
) -> RequestOutcome<DocumentDiagnosticReportResult> {
    let uri = params.text_document.uri;
    let cache_key = format!("diagnostic:{}", uri.as_str());

    let request_token = token.clone();
    let result = analysis_query(
        context.analysis(),
        context.request_manager(),
        &cache_key,
        Some(token.clone()),
        move |analysis| {
            let file_id = analysis.get_file_id(&uri)?;
            let diagnostics = analysis.diagnose_file(file_id, token.clone());
            if diagnostics.is_none()
                && !token.is_cancelled()
                && !analysis.diagnostic_cancellation_token().is_cancelled()
            {
                return None;
            }
            Some(diagnostics)
        },
    )
    .await
    .map(|diagnostics| {
        diagnostics.map(|diagnostics| {
            DocumentDiagnosticReport::Full(RelatedFullDocumentDiagnosticReport {
                related_documents: None,
                full_document_diagnostic_report: FullDocumentDiagnosticReport {
                    result_id: None,
                    items: diagnostics,
                },
            })
            .into()
        })
    });
    match result {
        RequestOutcome::Ready(Some(report)) => RequestOutcome::Ready(report),
        RequestOutcome::Ready(None) => RequestOutcome::Cancelled(if request_token.is_cancelled() {
            CancelSource::Client
        } else {
            CancelSource::DiagnosticRefresh
        }),
        RequestOutcome::Cancelled(source) => RequestOutcome::Cancelled(source),
        RequestOutcome::Missing => RequestOutcome::Missing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::ServerContext;
    use std::sync::Arc;

    #[tokio::test]
    async fn disabled_diagnostics_do_not_request_a_retry() {
        let (server, _client) = lsp_server::Connection::memory();
        let context = ServerContext::new(Arc::new(server), Default::default());
        let snapshot = context.snapshot();
        let params: DocumentDiagnosticParams = serde_json::from_value(serde_json::json!({
            "textDocument": {"uri": "file:///C:/ws/disabled.lua"}
        }))
        .unwrap();
        snapshot
            .analysis()
            .update(|analysis| {
                let mut config = emmylua_code_analysis::Emmyrc::default();
                config.diagnostics.enable = false;
                analysis.update_config(Arc::new(config));
                analysis.update_file_by_uri(
                    &params.text_document.uri,
                    Some("local x = missing".into()),
                );
            })
            .await;
        let result = on_pull_document_diagnostic(snapshot, params, CancellationToken::new()).await;
        context.close().await;
        assert!(matches!(result, RequestOutcome::Missing), "{result:?}");
    }

    #[tokio::test]
    async fn pull_diagnostics_distinguish_cancellation_missing_files_and_recovery() {
        let (server, _client) = lsp_server::Connection::memory();
        let context = ServerContext::new(Arc::new(server), Default::default());
        let snapshot = context.snapshot();
        let params: DocumentDiagnosticParams = serde_json::from_value(serde_json::json!({
            "textDocument": {"uri": "file:///C:/ws/cancelled.lua"}
        }))
        .unwrap();
        let missing =
            on_pull_document_diagnostic(snapshot.clone(), params.clone(), CancellationToken::new())
                .await;
        snapshot
            .analysis()
            .update(|analysis| {
                analysis
                    .diagnostic
                    .enable_only(emmylua_code_analysis::DiagnosticCode::UndefinedGlobal);
                analysis.update_file_by_uri(
                    &params.text_document.uri,
                    Some("local x = missing".into()),
                );
            })
            .await;
        let client_token = CancellationToken::new();
        client_token.cancel();
        let client_cancelled =
            on_pull_document_diagnostic(snapshot.clone(), params.clone(), client_token).await;
        snapshot.analysis().with_snapshot(|analysis| {
            analysis.diagnostic_cancellation_token().cancel();
        });
        let request_token = CancellationToken::new();
        let refreshed =
            on_pull_document_diagnostic(snapshot.clone(), params.clone(), request_token.clone())
                .await;
        let request_was_cancelled = request_token.is_cancelled();
        snapshot.analysis().update(|_| ()).await;
        let recovered =
            on_pull_document_diagnostic(snapshot, params, CancellationToken::new()).await;
        context.close().await;
        assert!(matches!(missing, RequestOutcome::Missing), "{missing:?}");
        assert!(matches!(
            client_cancelled,
            RequestOutcome::Cancelled(CancelSource::Client)
        ));
        assert!(matches!(
            refreshed,
            RequestOutcome::Cancelled(CancelSource::DiagnosticRefresh)
        ));
        assert!(!request_was_cancelled);
        let RequestOutcome::Ready(DocumentDiagnosticReportResult::Report(
            DocumentDiagnosticReport::Full(report),
        )) = recovered
        else {
            panic!("expected fresh diagnostics: {recovered:?}");
        };
        assert!(!report.full_document_diagnostic_report.items.is_empty());
    }
}
