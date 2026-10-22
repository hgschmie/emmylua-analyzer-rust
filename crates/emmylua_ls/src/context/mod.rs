mod analysis_state;
mod client;
mod client_id;
mod diagnostic_service;
mod lsp_features;
mod pull_cache;
mod query_runner;
mod snapshot;
mod status_bar;
mod update_queue;
mod workspace_manager;
mod workspace_state;

pub use analysis_state::AnalysisState;
pub use client::ClientProxy;
pub use client_id::{ClientId, get_client_id};
pub use diagnostic_service::DiagnosticService;
pub use lsp_features::LspFeatures;
use lsp_server::{Connection, ErrorCode, RequestId, Response};
use lsp_types::ClientCapabilities;
pub use pull_cache::RequestManager;
pub use query_runner::{CancelSource, RequestOutcome, analysis_query, snapshot_query};
pub use snapshot::ServerContextSnapshot;
pub use status_bar::ProgressTask;
pub use status_bar::StatusBar;
use std::{collections::HashMap, future::Future, sync::Arc};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
pub use update_queue::{UpdateEvent, spawn_update_queue};
pub use workspace_manager::*;

use crate::context::snapshot::ServerContextInner;
use crate::util::catch_unwind;

pub struct ServerContext {
    cancellations: Arc<Mutex<HashMap<RequestId, CancellationToken>>>,
    inner: Arc<ServerContextInner>,
}

impl ServerContext {
    pub fn new(conn: Arc<Connection>, client_capabilities: ClientCapabilities) -> Self {
        let client = Arc::new(ClientProxy::new(conn));

        let analysis = Arc::new(AnalysisState::new());
        let lsp_features = Arc::new(LspFeatures::new(client_capabilities));
        let status_bar = Arc::new(StatusBar::new(
            client.clone(),
            lsp_features.supports_work_done_progress(),
        ));
        let file_diagnostic = Arc::new(DiagnosticService::new(
            analysis.clone(),
            status_bar.clone(),
            client.clone(),
        ));
        let workspace_manager = Arc::new(Mutex::new(WorkspaceManager::new(
            client.clone(),
            file_diagnostic.clone(),
            lsp_features.clone(),
        )));

        let (update_tx, update_rx) = tokio::sync::mpsc::unbounded_channel();
        let inner = Arc::new(ServerContextInner {
            analysis,
            client,
            file_diagnostic,
            workspace_manager,
            status_bar,
            lsp_features,
            request_manager: Arc::new(RequestManager::new()),
            update_tx,
        });
        spawn_update_queue(inner.clone(), update_rx);

        ServerContext {
            cancellations: Arc::new(Mutex::new(HashMap::new())),
            inner,
        }
    }

    pub fn snapshot(&self) -> ServerContextSnapshot {
        ServerContextSnapshot::new(self.inner.clone())
    }

    pub fn send(&self, response: Response) {
        self.inner.client.send_response(response);
    }

    pub async fn task<T, F, Fut>(&self, req_id: RequestId, exec: F)
    where
        T: serde::Serialize + Send + 'static,
        F: FnOnce(CancellationToken) -> Fut + Send + 'static,
        Fut: Future<Output = RequestOutcome<T>> + Send + 'static,
    {
        let cancel_token = CancellationToken::new();

        {
            let mut cancellations = self.cancellations.lock().await;
            cancellations.insert(req_id.clone(), cancel_token.clone());
        }

        let client = self.inner.client.clone();
        let cancellations = self.cancellations.clone();

        tokio::spawn(async move {
            // Catch panics so every request still gets a response.
            let res = catch_unwind(exec(cancel_token.clone())).await;

            let response = match res {
                Ok(RequestOutcome::Ready(value)) => Response::new_ok(req_id.clone(), value),
                Ok(RequestOutcome::Missing) => {
                    Response::new_ok(req_id.clone(), serde_json::Value::Null)
                }
                Ok(RequestOutcome::Cancelled(CancelSource::DiagnosticRefresh)) => Response {
                    id: req_id.clone(),
                    response_result: Err(lsp_server::ResponseError {
                        code: ErrorCode::ServerCancelled as i32,
                        message: "diagnostics preempted by an update".to_string(),
                        data: Some(serde_json::json!({"retriggerRequest": true})),
                    }),
                },
                Ok(RequestOutcome::Cancelled(CancelSource::Client)) => Response::new_err(
                    req_id.clone(),
                    ErrorCode::RequestCanceled as i32,
                    "cancel".to_string(),
                ),
                Err(_) => Response::new_err(
                    req_id.clone(),
                    ErrorCode::InternalError as i32,
                    "internal error".to_string(),
                ),
            };
            client.send_response(response);

            let mut cancellations = cancellations.lock().await;
            cancellations.remove(&req_id);
        });
    }

    pub async fn cancel(&self, req_id: RequestId) {
        let cancellations = self.cancellations.lock().await;
        if let Some(cancel_token) = cancellations.get(&req_id) {
            cancel_token.cancel();
        }
    }

    pub async fn close(&self) {
        {
            let mut workspace_manager = self.inner.workspace_manager.lock().await;
            workspace_manager.watcher = None;
        }
        let _ = self.inner.update_tx.send(UpdateEvent::Shutdown);
    }

    pub async fn send_response(&self, response: Response) {
        self.inner.client.on_response(response).await;
    }
}

#[cfg(test)]
mod diagnostic_cancellation_response_tests {
    use super::*;

    #[tokio::test]
    async fn preempted_diagnostic_requests_request_a_retry() {
        let (server, client) = Connection::memory();
        let context = ServerContext::new(Arc::new(server), ClientCapabilities::default());
        context
            .task::<(), _, _>(RequestId::from(1), |_| async {
                RequestOutcome::Cancelled(CancelSource::DiagnosticRefresh)
            })
            .await;
        let response = tokio::task::spawn_blocking(move || {
            client
                .receiver
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap()
        })
        .await
        .unwrap();
        let lsp_server::Message::Response(response) = response else {
            panic!("expected response")
        };
        let error = response.response_result.unwrap_err();
        assert_eq!(error.code, -32802);
        assert_eq!(
            error.data,
            Some(serde_json::json!({"retriggerRequest": true}))
        );
    }
}
