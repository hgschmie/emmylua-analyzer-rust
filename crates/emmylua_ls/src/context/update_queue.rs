//! # File update queue
//!
//! All filesystem changes are enqueued and processed serially by one worker, guaranteeing:
//! - open/change/close/watch/rename ordering;
//! - the notification loop never blocks;
//! - `$/cancelRequest` is always handled promptly.

use std::sync::Arc;

use lsp_types::{
    DidChangeTextDocumentParams, DidChangeWatchedFilesParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DidSaveTextDocumentParams, RenameFilesParams,
};
use tokio::sync::{mpsc, oneshot};

use crate::handlers;

use super::snapshot::{ServerContextInner, ServerContextSnapshot};

pub enum UpdateEvent {
    DidOpen(DidOpenTextDocumentParams),
    DidChange(DidChangeTextDocumentParams),
    DidClose(DidCloseTextDocumentParams),
    DidSave(DidSaveTextDocumentParams),
    DidChangeWatchedFiles(DidChangeWatchedFilesParams),
    DidRenameFiles(RenameFilesParams),
    /// Acknowledge preceding updates without running the request on this worker.
    Barrier(oneshot::Sender<()>),
    Shutdown,
}

pub fn spawn_update_queue(
    inner: Arc<ServerContextInner>,
    mut rx: mpsc::UnboundedReceiver<UpdateEvent>,
) {
    tokio::spawn(async move {
        loop {
            let event = tokio::select! {
                biased;
                _ = inner.shutdown.cancelled() => break,
                event = rx.recv() => match event {
                    Some(event) => event,
                    None => break,
                },
            };
            let snapshot = ServerContextSnapshot::new(inner.clone());
            match event {
                UpdateEvent::DidOpen(params) => {
                    handlers::process_did_open_text_document(snapshot, params).await;
                }
                UpdateEvent::DidChange(params) => {
                    handlers::process_did_change_text_document(snapshot, params).await;
                }
                UpdateEvent::DidSave(params) => {
                    handlers::process_did_save_text_document(snapshot, params).await;
                }
                UpdateEvent::DidClose(params) => {
                    handlers::process_did_close_document(snapshot, params).await;
                }
                UpdateEvent::DidChangeWatchedFiles(params) => {
                    handlers::process_did_change_watched_files(snapshot, params).await;
                }
                UpdateEvent::DidRenameFiles(params) => {
                    handlers::process_did_rename_files_handler(snapshot, params).await;
                }
                UpdateEvent::Barrier(ready) => {
                    // The requester may already have been cancelled.
                    let _ = ready.send(());
                }
                UpdateEvent::Shutdown => {
                    break;
                }
            }
        }
    });
}
