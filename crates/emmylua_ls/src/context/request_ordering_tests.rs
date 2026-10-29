use super::*;
use lsp_server::{Message, Request};
use lsp_types::{
    ApplyWorkspaceEditParams, DidChangeTextDocumentParams, FileRename, RenameFilesParams,
    ShowMessageRequestParams, TextDocumentContentChangeEvent, Uri, VersionedTextDocumentIdentifier,
};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::Duration,
};
use tokio::{sync::oneshot, time::timeout};

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        loop {
            let path = std::env::temp_dir().join(format!(
                "emmylua-request-ordering-{}-{}",
                std::process::id(),
                NEXT_ID.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("failed to create test directory: {error}"),
            }
        }
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

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

async fn client_request(client: &Connection, method: &str) -> Request {
    timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(Message::Request(request)) = client.receiver.try_recv() {
                assert_eq!(request.method, method);
                return request;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("outgoing client request")
}

#[tokio::test]
async fn request_waits_for_all_preceding_edits() {
    let (context, client, uri) = setup().await;
    // Queue both edits and the request before releasing the workspace. The
    // manually polled future test checks the wait without a wall-clock deadline.
    let held = context.inner.workspace_manager.lock().await;
    change(&context, &uri, 1, "local intermediate = 2");
    change(&context, &uri, 2, "local after = 3");
    read(&context, &uri, 1).await;
    drop(held);
    assert_eq!(
        response(&client).await.response_result.unwrap(),
        "local after = 3"
    );
    context.close().await;
}

#[tokio::test]
async fn request_future_waits_for_barrier_acknowledgement() {
    use std::{future::Future, task::Poll};

    let (ready, updates_applied) = oneshot::channel();
    let called = Arc::new(AtomicBool::new(false));
    let executed = called.clone();
    let mut request = Box::pin(execute_after_updates(
        updates_applied,
        CancellationToken::new(),
        move |_| async move {
            executed.store(true, Ordering::SeqCst);
            RequestOutcome::Ready(1)
        },
    ));
    std::future::poll_fn(|cx| {
        assert!(request.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    assert!(!called.load(Ordering::SeqCst));
    ready.send(()).unwrap();
    assert!(matches!(request.await, Ok(RequestOutcome::Ready(1))));
    assert!(called.load(Ordering::SeqCst));
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

#[tokio::test]
async fn unavailable_update_worker_returns_internal_error() {
    // Cover both a refused send and an accepted barrier abandoned by shutdown.
    for already_stopped in [true, false] {
        let (context, client, _) = setup().await;
        context.inner.update_tx.send(UpdateEvent::Shutdown).unwrap();
        if already_stopped {
            context.inner.update_tx.closed().await;
        }
        let called = Arc::new(AtomicBool::new(false));
        let executed = called.clone();
        context
            .task(1.into(), move |_| async move {
                executed.store(true, Ordering::SeqCst);
                RequestOutcome::Ready(1)
            })
            .await;
        let error = response(&client).await.response_result.unwrap_err();
        assert_eq!(error.code, ErrorCode::InternalError as i32);
        assert!(!called.load(Ordering::SeqCst));
        assert!(context.cancellations.lock().await.is_empty());
        context.close().await;
    }
}

#[tokio::test]
async fn rename_prompt_does_not_block_updates_or_requests() {
    check_rename_client_interaction(true).await;
}

#[tokio::test]
async fn rename_edit_acknowledgement_does_not_block_updates_or_requests() {
    check_rename_client_interaction(false).await;
}

async fn check_rename_client_interaction(wait_at_prompt: bool) {
    let directory = TestDirectory::new();
    let old_path = directory.0.join("old.lua");
    let new_path = directory.0.join("new.lua");
    let consumer_path = directory.0.join("consumer.lua");
    std::fs::write(&old_path, "return 1").unwrap();
    std::fs::write(&consumer_path, "local value = require('old')").unwrap();
    let old_uri = emmylua_code_analysis::file_path_to_uri(&old_path).unwrap();
    let new_uri = emmylua_code_analysis::file_path_to_uri(&new_path).unwrap();
    let consumer_uri = emmylua_code_analysis::file_path_to_uri(&consumer_path).unwrap();
    let (context, client, _) = setup().await;
    context
        .inner
        .analysis
        .update(|analysis| {
            analysis.add_main_workspace(directory.0.clone());
            analysis.update_file_by_uri(&old_uri, Some("return 1".into()));
            analysis.update_file_by_uri(&consumer_uri, Some("local value = require('old')".into()));
        })
        .await;
    std::fs::rename(&old_path, &new_path).unwrap();
    context
        .inner
        .update_tx
        .send(UpdateEvent::DidRenameFiles(RenameFilesParams {
            files: vec![FileRename {
                old_uri: old_uri.as_str().into(),
                new_uri: new_uri.as_str().into(),
            }],
        }))
        .unwrap();
    let prompt = client_request(&client, "window/showMessageRequest").await;
    let prompt_params: ShowMessageRequestParams = serde_json::from_value(prompt.params).unwrap();
    let action = prompt_params.actions.unwrap().into_iter().next().unwrap();

    if wait_at_prompt {
        // Leave the user prompt unanswered while a later edit and query complete.
        change(&context, &new_uri, 1, "return 2");
        read(&context, &new_uri, 10).await;
        let edited = response(&client).await;
        assert_eq!(edited.id, 10.into());
        assert_eq!(edited.response_result.unwrap(), "return 2");
    }
    context
        .send_response(Response::new_ok(
            prompt.id,
            serde_json::to_value(action).unwrap(),
        ))
        .await;

    let apply = client_request(&client, "workspace/applyEdit").await;
    let apply_params: ApplyWorkspaceEditParams = serde_json::from_value(apply.params).unwrap();
    let changes = apply_params.edit.changes.unwrap();
    assert_eq!(changes[&consumer_uri][0].new_text, "'new'");

    if !wait_at_prompt {
        // Waiting for the client's edit acknowledgement must not hold the queue either.
        change(&context, &new_uri, 1, "return 3");
        read(&context, &new_uri, 11).await;
        let edited = response(&client).await;
        assert_eq!(edited.id, 11.into());
        assert_eq!(edited.response_result.unwrap(), "return 3");
    }
    context
        .send_response(Response::new_ok(
            apply.id,
            serde_json::json!({ "applied": true }),
        ))
        .await;
    context.close().await;
}
