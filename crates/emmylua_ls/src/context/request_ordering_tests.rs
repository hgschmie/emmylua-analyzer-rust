use super::*;
use lsp_server::Message;
use lsp_types::{
    DidChangeTextDocumentParams, TextDocumentContentChangeEvent, Uri,
    VersionedTextDocumentIdentifier,
};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tokio::{sync::oneshot, time::timeout};

async fn setup() -> (Arc<ServerContext>, Connection, Uri) {
    let (server, client) = Connection::memory();
    let capabilities = serde_json::from_value(serde_json::json!({
        "textDocument": { "diagnostic": {} }
    }))
    .unwrap();
    let context = Arc::new(ServerContext::new(Arc::new(server), capabilities));
    let uri: Uri = "file:///virtual/request-ordering.lua".parse().unwrap();
    context
        .inner
        .analysis
        .update(|analysis| {
            analysis.update_file_by_uri(&uri, Some("local before = 1".to_string()));
        })
        .await;
    (context, client, uri)
}

fn change(context: &ServerContext, uri: &Uri, version: i32, text: &str) {
    context
        .inner
        .update_tx
        .send(UpdateEvent::DidChange(DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: uri.clone(),
                version,
            },
            content_changes: vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: text.to_string(),
            }],
        }))
        .unwrap();
}

async fn read(context: &ServerContext, uri: &Uri, id: i32) {
    let snapshot = context.snapshot();
    let uri = uri.clone();
    context
        .task(id.into(), move |_| async move {
            snapshot
                .analysis()
                .query_blocking(move |analysis| {
                    let file_id = analysis.get_file_id(&uri)?;
                    analysis
                        .db
                        .get_file_text(file_id)
                        .map(|text| text.to_string())
                })
                .await
        })
        .await;
}

async fn response(client: &Connection) -> Response {
    timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(message) = client.receiver.try_recv() {
                if let Message::Response(response) = message {
                    return response;
                }
            } else {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        }
    })
    .await
    .expect("request response")
}

#[tokio::test]
async fn request_waits_for_all_preceding_edits() {
    let (context, client, uri) = setup().await;
    // Block the update worker before it can apply either edit, without blocking
    // analysis reads. The old dispatcher can return the initial text here.
    let held = context.inner.workspace_manager.lock().await;
    change(&context, &uri, 1, "local intermediate = 2");
    change(&context, &uri, 2, "local after = 3");
    read(&context, &uri, 1).await;
    assert!(
        timeout(Duration::from_millis(100), response(&client))
            .await
            .is_err(),
        "request overtook preceding edits"
    );
    drop(held);
    assert_eq!(
        response(&client).await.response_result.unwrap(),
        "local after = 3"
    );
    context.close().await;
}

#[tokio::test]
async fn cancelled_request_does_not_run_while_edit_is_blocked() {
    let (context, client, uri) = setup().await;
    let held = context.inner.workspace_manager.lock().await;
    change(&context, &uri, 1, "local after = 2");
    let called = Arc::new(AtomicBool::new(false));
    let executed = called.clone();
    context
        .task(1.into(), move |_| async move {
            executed.store(true, Ordering::SeqCst);
            RequestOutcome::Ready(1)
        })
        .await;
    context.cancel(1.into()).await;
    let error = response(&client).await.response_result.unwrap_err();
    assert_eq!(error.code, ErrorCode::RequestCanceled as i32);
    assert!(!called.load(Ordering::SeqCst));
    drop(held);
    // Cancelling the earlier request must not strand the update worker.
    read(&context, &uri, 2).await;
    assert_eq!(
        response(&client).await.response_result.unwrap(),
        "local after = 2"
    );
    context.close().await;
}

#[tokio::test]
async fn request_does_not_wait_for_later_edits() {
    let (context, client, uri) = setup().await;
    let held = context.inner.workspace_manager.lock().await;
    read(&context, &uri, 1).await;
    change(&context, &uri, 1, "local later = 2");
    assert_eq!(
        response(&client).await.response_result.unwrap(),
        "local before = 1"
    );
    drop(held);
    read(&context, &uri, 2).await;
    assert_eq!(
        response(&client).await.response_result.unwrap(),
        "local later = 2"
    );
    context.close().await;
}

#[tokio::test]
async fn pending_request_does_not_block_updates_or_other_requests() {
    let (context, client, uri) = setup().await;
    let (started_tx, started_rx) = oneshot::channel();
    let (finish_tx, finish_rx) = oneshot::channel();
    context
        .task(1.into(), move |_| async move {
            started_tx.send(()).unwrap();
            finish_rx.await.unwrap();
            RequestOutcome::Ready("finished")
        })
        .await;
    timeout(Duration::from_secs(5), started_rx)
        .await
        .unwrap()
        .unwrap();
    change(&context, &uri, 1, "local while_pending = 2");
    read(&context, &uri, 2).await;
    let second = response(&client).await;
    assert_eq!(second.id, 2.into());
    assert_eq!(second.response_result.unwrap(), "local while_pending = 2");
    finish_tx.send(()).unwrap();
    assert_eq!(response(&client).await.id, 1.into());
    context.close().await;
}

#[tokio::test]
async fn close_cancels_requests_waiting_for_updates() {
    let (context, client, uri) = setup().await;
    let held = context.inner.workspace_manager.lock().await;
    change(&context, &uri, 1, "local after = 2");
    let called = Arc::new(AtomicBool::new(false));
    let executed = called.clone();
    context
        .task(1.into(), move |_| async move {
            executed.store(true, Ordering::SeqCst);
            RequestOutcome::Ready(1)
        })
        .await;
    let closing = context.clone();
    let closed = tokio::spawn(async move { closing.close().await });
    let error = response(&client).await.response_result.unwrap_err();
    assert_eq!(error.code, ErrorCode::RequestCanceled as i32);
    assert!(!called.load(Ordering::SeqCst));
    drop(held);
    timeout(Duration::from_secs(5), closed)
        .await
        .unwrap()
        .unwrap();
}
