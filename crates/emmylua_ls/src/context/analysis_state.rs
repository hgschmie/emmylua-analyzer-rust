//! # AnalysisState — shared analysis state protected by a read-write lock
//!
//! Reads are served directly from the live `EmmyLuaAnalysis` under a
//! read lock; writes take the write lock exclusively. We do not clone the whole
//! analysis to run queries.

use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use emmylua_code_analysis::EmmyLuaAnalysis;

use crate::context::RequestOutcome;

pub struct AnalysisState {
    inner: Arc<RwLock<EmmyLuaAnalysis>>,
    diagnostic_cancellation: Mutex<CancellationToken>,
    blocking_permits: Arc<Semaphore>,
    gate: Arc<tokio::sync::RwLock<()>>,
}

impl AnalysisState {
    pub fn new() -> Self {
        let analysis = EmmyLuaAnalysis::new();
        let diagnostic_cancellation = Mutex::new(analysis.diagnostic_cancellation_token());
        Self {
            inner: Arc::new(RwLock::new(analysis)),
            diagnostic_cancellation,
            gate: Arc::new(tokio::sync::RwLock::new(())),
            blocking_permits: Arc::new(Semaphore::new(
                std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(4),
            )),
        }
    }

    pub fn with_snapshot<T>(&self, f: impl FnOnce(&EmmyLuaAnalysis) -> T) -> Option<T> {
        let analysis = self.read();
        Some(f(&analysis))
    }

    pub fn try_with_snapshot<R>(&self, f: impl FnOnce(&EmmyLuaAnalysis) -> Option<R>) -> Option<R> {
        let analysis = self.read();
        f(&analysis)
    }

    /// Logical parallelism used by the analysis blocking pool.
    pub fn analysis_parallelism() -> usize {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
    }

    pub async fn run_blocking<R, F>(&self, f: F) -> Option<R>
    where
        R: Send + 'static,
        F: FnOnce(&EmmyLuaAnalysis) -> Option<R> + Send + 'static,
    {
        let _permit = self.blocking_permits.clone().acquire_owned().await.ok()?;
        let inner = self.inner.clone();
        let gate = self.gate.clone().read_owned().await;
        let result = tokio::task::spawn_blocking(move || {
            let analysis = inner
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            drop(gate);
            f(&analysis)
        })
        .await;
        match result {
            Ok(value) => value,
            Err(err) => {
                if err.is_panic() {
                    std::panic::resume_unwind(err.into_panic());
                }
                None
            }
        }
    }

    pub async fn query_blocking<R, F>(&self, f: F) -> RequestOutcome<R>
    where
        R: Send + 'static,
        F: FnOnce(&EmmyLuaAnalysis) -> Option<R> + Send + 'static,
    {
        match self.run_blocking(f).await {
            Some(value) => RequestOutcome::Ready(value),
            None => RequestOutcome::Missing,
        }
    }

    pub async fn update<R>(&self, f: impl FnOnce(&mut EmmyLuaAnalysis) -> R) -> R {
        let inner = self.inner.clone();
        let gate = self.gate.clone().write_owned().await;
        // Cancel readers before waiting for their analysis read locks.
        self.diagnostic_cancellation
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .cancel();
        let run = move || {
            let mut analysis = inner
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *self
                .diagnostic_cancellation
                .lock()
                .unwrap_or_else(|p| p.into_inner()) = analysis.renew_diagnostic_cancellation();
            drop(gate);
            f(&mut analysis)
        };
        if tokio::runtime::Handle::try_current()
            .map(|handle| handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread)
            .unwrap_or(false)
        {
            tokio::task::block_in_place(run)
        } else {
            run()
        }
    }

    fn read(&self) -> RwLockReadGuard<'_, EmmyLuaAnalysis> {
        self.inner
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Default for AnalysisState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[tokio::test]
    async fn update_broadcasts_analysis_state() {
        let state = AnalysisState::new();
        let uri = lsp_types::Uri::from_str("file:///C:/ws/snapshot.lua").unwrap();

        state
            .update(|analysis| {
                analysis.update_file_by_uri(&uri, Some("local x = 1".to_string()));
            })
            .await;

        // Read path uses the live analysis under a read lock.
        let result = state.try_with_snapshot(|analysis| {
            let file_id = analysis.get_file_id(&uri)?;
            let model = analysis.semantic_model(file_id);
            let decls = model.decls()?;
            Some((decls.len(), decls[0].name.as_str().to_string()))
        });
        assert_eq!(result, Some((1, "x".to_string())));
    }
    #[tokio::test]
    async fn blocking_query_and_update_complete() {
        let state = Arc::new(AnalysisState::new());
        let read_state = state.clone();
        let read = tokio::spawn(async move { read_state.run_blocking(|_| Some(1)).await });
        let write_state = state.clone();
        let write = tokio::spawn(async move { write_state.update(|_| 2).await });
        assert_eq!(read.await.unwrap(), Some(1));
        assert_eq!(write.await.unwrap(), 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn waiting_update_preempts_diagnostics_and_restores_fresh_checks() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::time::Duration;
        use tokio_util::sync::CancellationToken;

        let state = Arc::new(AnalysisState::new());
        let uri = lsp_types::Uri::from_str("file:///C:/ws/preempt.lua").unwrap();
        let file_id = state
            .update(|analysis| {
                analysis
                    .diagnostic
                    .enable_only(emmylua_code_analysis::DiagnosticCode::UndefinedGlobal);
                analysis.update_file_by_uri(&uri, Some("local x = missing_before_edit".into()));
                analysis.get_file_id(&uri).unwrap()
            })
            .await;
        let request_token = CancellationToken::new();
        let before = state
            .try_with_snapshot(|analysis| analysis.diagnose_file(file_id, request_token.clone()))
            .unwrap();
        assert!(
            before
                .iter()
                .any(|d| d.message.contains("missing_before_edit"))
        );

        let stop = Arc::new(AtomicBool::new(false));
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let read_state = state.clone();
        let read_stop = stop.clone();
        let read_token = request_token.clone();
        let reader = tokio::spawn(async move {
            read_state
                .run_blocking(move |analysis| {
                    let _ = started_tx.send(());
                    release_rx.recv_timeout(Duration::from_secs(10)).ok()?;
                    // Retain the same read lock until the host cancels diagnostics.
                    // The watchdog only bounds a broken implementation; it never cancels a token.
                    let mut result = analysis.diagnose_file(file_id, read_token.clone());
                    while result.is_some() && !read_stop.load(Ordering::Acquire) {
                        std::thread::yield_now();
                        result = analysis.diagnose_file(file_id, read_token.clone());
                    }
                    Some(result)
                })
                .await
        });
        let started = matches!(
            tokio::time::timeout(Duration::from_secs(5), started_rx).await,
            Ok(Ok(()))
        );
        let write_state = state.clone();
        let writer = tokio::spawn(async move {
            write_state
                .update(|analysis| {
                    analysis.update_file_by_uri(&uri, Some("local x = missing_after_edit".into()));
                })
                .await;
        });
        // run_blocking drops its gate before entering the callback, so only the
        // writer can hold it here while waiting for the callback's read lock.
        let writer_waiting = tokio::time::timeout(Duration::from_secs(5), async {
            while state.gate.try_read().is_ok() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .is_ok();
        let (watchdog_tx, watchdog_rx) = std::sync::mpsc::channel();
        let watchdog = std::thread::spawn(move || {
            if watchdog_rx.recv_timeout(Duration::from_secs(2)).is_err() {
                stop.store(true, Ordering::Release);
            }
        });
        let _ = release_tx.send(());
        let result = reader.await;
        let _ = watchdog_tx.send(());
        let watchdog_result = watchdog.join();
        let writer_result = writer.await;
        // Complete both workers and the watchdog before making failure assertions.
        assert!(started, "analysis reader did not start");
        assert!(writer_waiting, "writer did not reach the gate");
        watchdog_result.unwrap();
        writer_result.unwrap();
        assert!(!request_token.is_cancelled());
        assert_eq!(
            result.unwrap(),
            Some(None),
            "the waiting update must cancel diagnostics"
        );
        let after = state
            .try_with_snapshot(|analysis| analysis.diagnose_file(file_id, request_token.clone()))
            .unwrap();
        assert!(
            after
                .iter()
                .any(|d| d.message.contains("missing_after_edit"))
        );
    }
}
