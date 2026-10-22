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
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn update_cancels_diagnostics_before_waiting_for_the_read_lock() {
        let state = Arc::new(AnalysisState::new());
        let reader = state.clone();
        let (started, ready) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            reader
                .run_blocking(move |analysis| {
                    let token = analysis.diagnostic_cancellation_token();
                    started.send(()).unwrap();
                    while !token.is_cancelled() {
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                    Some(())
                })
                .await
        });
        ready.await.unwrap();
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            state.update(|analysis| {
                assert!(!analysis.diagnostic_cancellation_token().is_cancelled());
            }),
        )
        .await
        .expect("writer must preempt diagnostic readers");
        assert_eq!(task.await.unwrap(), Some(()));
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
}
