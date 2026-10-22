//! # Unified analysis query runner
//!
//! All pull requests go through here:
//! - completed normally → `Ready`
//! - genuinely no data → `Missing`
//! - external cancellation / replaced by latest-wins → `Cancelled(Client)`

use super::{AnalysisState, RequestManager};
use emmylua_code_analysis::EmmyLuaAnalysis;
use tokio_util::sync::CancellationToken;

/// Cancellation source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelSource {
    /// Client `$/cancelRequest`, or a newer same-key request replacing the old one.
    Client,
    /// A diagnostic computation was preempted by an analysis update.
    DiagnosticRefresh,
}

/// Unified request result.
#[derive(Debug)]
pub enum RequestOutcome<T> {
    Ready(T),
    Cancelled(CancelSource),
    Missing,
}

/// Unified executor for pull requests with PullCache.
pub async fn analysis_query<T, F>(
    analysis: &AnalysisState,
    request_manager: &RequestManager,
    key: &str,
    cancel_token: Option<CancellationToken>,
    compute: F,
) -> RequestOutcome<T>
where
    T: Send + Sync + 'static,
    F: FnOnce(&EmmyLuaAnalysis) -> Option<T> + Send + 'static,
{
    if let Some(token) = &cancel_token {
        request_manager.begin(key, token.clone()).await;
    }
    let key_owner = request_manager.lock_for(key).await;
    let _guard = key_owner.lock().await;

    if let Some(token) = &cancel_token
        && token.is_cancelled()
    {
        return RequestOutcome::Cancelled(CancelSource::Client);
    }

    match analysis.query_blocking(compute).await {
        RequestOutcome::Ready(value) => RequestOutcome::Ready(value),
        RequestOutcome::Missing => {
            if let Some(token) = &cancel_token
                && token.is_cancelled()
            {
                RequestOutcome::Cancelled(CancelSource::Client)
            } else {
                RequestOutcome::Missing
            }
        }
        RequestOutcome::Cancelled(source) => RequestOutcome::Cancelled(source),
    }
}

/// Unified executor for plain snapshot queries without PullCache.
pub async fn snapshot_query<T, F>(
    analysis: &AnalysisState,
    cancel_token: CancellationToken,
    compute: F,
) -> RequestOutcome<T>
where
    T: Send + Sync + 'static,
    F: FnOnce(&EmmyLuaAnalysis) -> Option<T> + Send + 'static,
{
    if cancel_token.is_cancelled() {
        return RequestOutcome::Cancelled(CancelSource::Client);
    }

    match analysis.query_blocking(compute).await {
        RequestOutcome::Ready(value) => RequestOutcome::Ready(value),
        RequestOutcome::Missing => {
            if cancel_token.is_cancelled() {
                RequestOutcome::Cancelled(CancelSource::Client)
            } else {
                RequestOutcome::Missing
            }
        }
        RequestOutcome::Cancelled(source) => RequestOutcome::Cancelled(source),
    }
}
