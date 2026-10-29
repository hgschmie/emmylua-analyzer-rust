use emmylua_code_analysis::{Emmyrc, read_file_with_encoding, uri_to_file_path};
use lsp_types::{
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DidSaveTextDocumentParams,
};
use std::sync::Arc;

use crate::context::{ServerContextSnapshot, UpdateEvent, WorkspaceDiagnosticLevel};

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
    _: DidSaveTextDocumentParams,
) -> Option<()> {
    let emmyrc = context
        .analysis()
        .with_snapshot(|analysis| analysis.get_emmyrc())
        .unwrap_or_else(|| Arc::new(Emmyrc::default()));
    if !emmyrc.workspace.enable_reindex {
        if context.lsp_features().supports_workspace_diagnostic() {
            context
                .file_diagnostic()
                .cancel_workspace_diagnostic()
                .await;
            let workspace_manager = context.workspace_manager().lock().await;
            workspace_manager.update_workspace_version(WorkspaceDiagnosticLevel::Slow, true);
        }

        return Some(());
    }

    // let mut duration = emmyrc.workspace.reindex_duration;
    // // if duration is less than 1000ms, set it to 1000ms
    // if duration < 1000 {
    //     duration = 1000;
    // }
    // let workspace = context.workspace_manager().lock().await;
    // workspace
    //     .reindex_workspace(Duration::from_millis(duration))
    //     .await;
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
    let was_open = workspace.is_open_file(uri);
    workspace.close_open_file(&params.text_document.uri);
    drop(workspace);
    let lsp_features = context.lsp_features();

    // If the closed file no longer exists, remove it.
    if let Some(file_path) = uri_to_file_path(uri)
        && !file_path.exists()
    {
        context
            .analysis()
            .update(|analysis| analysis.remove_file_by_uri(uri))
            .await;

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
        context
            .analysis()
            .update(|analysis| analysis.remove_file_by_uri(uri))
            .await;

        if !lsp_features.supports_pull_diagnostic() {
            context
                .file_diagnostic()
                .clear_push_file_diagnostics(uri.clone());
        }
    }

    // Closing hands authority back to disk. Changes observed while the buffer
    // was open were deliberately ignored by the file watcher.
    if was_open
        && file_exists
        && let Some(path) = uri_to_file_path(uri)
        && let Some(config) = context
            .analysis()
            .with_snapshot(|analysis| analysis.get_emmyrc())
    {
        enum ReloadResult {
            Content(String),
            Missing,
            Unreadable,
        }

        let encoding = config.workspace.encoding.clone();
        let text = match tokio::task::spawn_blocking(move || {
            if let Some(text) = read_file_with_encoding(&path, &encoding) {
                ReloadResult::Content(text)
            } else if matches!(
                std::fs::metadata(&path),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound
            ) {
                ReloadResult::Missing
            } else {
                ReloadResult::Unreadable
            }
        })
        .await
        {
            Ok(ReloadResult::Content(text)) => text,
            Ok(ReloadResult::Missing) => {
                let removed = context
                    .analysis()
                    .update(|analysis| analysis.remove_file_by_uri(uri))
                    .await;
                if removed.is_some() {
                    context
                        .workspace_manager()
                        .lock()
                        .await
                        .update_workspace_version(WorkspaceDiagnosticLevel::Slow, true);
                }
                if !lsp_features.supports_pull_diagnostic() {
                    context
                        .file_diagnostic()
                        .clear_push_file_diagnostics(uri.clone());
                }
                return Some(());
            }
            Ok(ReloadResult::Unreadable) => {
                log::warn!(
                    "Could not reload closed document {uri:?} from disk; keeping previous analysis"
                );
                return Some(());
            }
            Err(error) => {
                log::warn!("Failed to reload closed document {uri:?}: {error}");
                return Some(());
            }
        };
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
            context
                .workspace_manager()
                .lock()
                .await
                .update_workspace_version(WorkspaceDiagnosticLevel::Slow, true);
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
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            static NEXT_ID: AtomicU64 = AtomicU64::new(0);
            loop {
                let path = std::env::temp_dir().join(format!(
                    "emmylua-close-{}-{}",
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

    #[tokio::test]
    async fn closing_document_reloads_disk_and_preserves_untracked_files() {
        let directory = TestDirectory::new();
        let file = directory.0.join("dependency.lua");
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
        assert_eq!(
            snapshot
                .workspace_manager()
                .lock()
                .await
                .get_workspace_version(),
            1
        );
        // Closing an unchanged buffer must not invalidate workspace diagnostics again.
        snapshot
            .workspace_manager()
            .lock()
            .await
            .sync_open_file(uri.clone(), "local disk = 1".into());
        process_did_close_document(
            snapshot.clone(),
            DidCloseTextDocumentParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
            },
        )
        .await;
        assert_eq!(
            snapshot
                .workspace_manager()
                .lock()
                .await
                .get_workspace_version(),
            1
        );
        // Files which were never part of this server must not be imported by closing them.
        let other = directory.0.join("untracked.lua");
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
        // Deleted files are removed whether the close was genuine or redundant.
        for was_open in [true, false] {
            std::fs::write(&file, "local disk = 1").unwrap();
            snapshot
                .analysis()
                .update(|analysis| {
                    analysis.update_file_by_uri(&uri, Some("local disk = 1".into()));
                })
                .await;
            if was_open {
                snapshot
                    .workspace_manager()
                    .lock()
                    .await
                    .sync_open_file(uri.clone(), "local disk = 1".into());
            }
            assert_eq!(
                snapshot.workspace_manager().lock().await.is_open_file(&uri),
                was_open
            );
            std::fs::remove_file(&file).unwrap();
            process_did_close_document(
                snapshot.clone(),
                DidCloseTextDocumentParams {
                    text_document: TextDocumentIdentifier { uri: uri.clone() },
                },
            )
            .await;
            assert!(!snapshot.workspace_manager().lock().await.is_open_file(&uri));
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
        }
    }

    #[tokio::test]
    async fn closing_unopened_document_does_not_reload_disk() {
        let directory = TestDirectory::new();
        let file = directory.0.join("unopened.lua");
        std::fs::write(&file, "local disk = 1").unwrap();
        let uri = emmylua_code_analysis::file_path_to_uri(&file).unwrap();
        let (server, _client) = Connection::memory();
        let context = ServerContext::new(Arc::new(server), ClientCapabilities::default());
        let snapshot = context.snapshot();
        snapshot
            .analysis()
            .update(|analysis| {
                analysis.update_file_by_uri(&uri, Some("local tracked = 2".into()));
            })
            .await;
        process_did_close_document(
            snapshot.clone(),
            DidCloseTextDocumentParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
            },
        )
        .await;
        snapshot.analysis().with_snapshot(|analysis| {
            let file_id = analysis.get_file_id(&uri).unwrap();
            assert_eq!(
                analysis.db.get_file_text(file_id),
                Some("local tracked = 2")
            );
        });
        assert_eq!(
            snapshot
                .workspace_manager()
                .lock()
                .await
                .get_workspace_version(),
            0
        );
    }

    #[tokio::test]
    async fn closing_document_preserves_analysis_when_disk_read_fails() {
        let directory = TestDirectory::new();
        let file = directory.0.join("unreadable.lua");
        // A directory exists but cannot be read as file content on any supported platform.
        std::fs::create_dir(&file).unwrap();
        let uri = emmylua_code_analysis::file_path_to_uri(&file).unwrap();
        let (server, _client) = Connection::memory();
        let context = ServerContext::new(Arc::new(server), ClientCapabilities::default());
        let snapshot = context.snapshot();
        snapshot
            .analysis()
            .update(|analysis| {
                analysis.update_file_by_uri(&uri, Some("local buffer = 2".into()));
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
            assert_eq!(analysis.db.get_file_text(file_id), Some("local buffer = 2"));
        });
        assert_eq!(
            snapshot
                .workspace_manager()
                .lock()
                .await
                .get_workspace_version(),
            0
        );
    }

    #[test]
    fn closing_document_removes_file_deleted_before_disk_read() {
        use std::future::Future;
        use std::task::Poll;
        use std::time::Duration;

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(1)
            .build()
            .unwrap();
        runtime.block_on(async {
            let directory = TestDirectory::new();
            let file = directory.0.join("deleted-before-read.lua");
            std::fs::write(&file, "local disk = 1").unwrap();
            let uri = emmylua_code_analysis::file_path_to_uri(&file).unwrap();
            let (server, client) = Connection::memory();
            let context = ServerContext::new(Arc::new(server), ClientCapabilities::default());
            let snapshot = context.snapshot();
            snapshot
                .analysis()
                .update(|analysis| {
                    analysis.update_file_by_uri(&uri, Some("local buffer = 2".into()));
                })
                .await;
            snapshot
                .workspace_manager()
                .lock()
                .await
                .sync_open_file(uri.clone(), "local buffer = 2".into());

            // Hold the sole blocking worker so deletion happens after the existence
            // check but before the queued disk read, without relying on sleeps.
            let (started_tx, started_rx) = tokio::sync::oneshot::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel();
            let blocker = tokio::task::spawn_blocking(move || {
                started_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(10))
            });
            started_rx.await.unwrap();
            let mut close = Box::pin(process_did_close_document(
                snapshot.clone(),
                DidCloseTextDocumentParams {
                    text_document: TextDocumentIdentifier { uri: uri.clone() },
                },
            ));
            std::future::poll_fn(|cx| {
                assert!(close.as_mut().poll(cx).is_pending());
                Poll::Ready(())
            })
            .await;
            assert!(!snapshot.workspace_manager().lock().await.is_open_file(&uri));
            std::fs::remove_file(&file).unwrap();
            release_tx.send(()).unwrap();
            close.await;
            blocker.await.unwrap().unwrap();

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
            assert_eq!(
                snapshot
                    .workspace_manager()
                    .lock()
                    .await
                    .get_workspace_version(),
                1
            );
            assert!(client.receiver.try_iter().any(|message| {
                let lsp_server::Message::Notification(notification) = message else {
                    return false;
                };
                notification.method == "textDocument/publishDiagnostics"
                    && serde_json::from_value::<lsp_types::PublishDiagnosticsParams>(
                        notification.params,
                    )
                    .is_ok_and(|params| params.uri == uri && params.diagnostics.is_empty())
            }));
        });
    }
}
