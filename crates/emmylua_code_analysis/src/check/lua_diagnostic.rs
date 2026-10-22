//! # LuaDiagnostic — diagnostic entry point (pure semantic path since M2)
//!
//! As of M2: `diagnose_file` only runs `analysis.diagnose_file_with_config` (the full `check/` checker set).
//! The old checker chain (`diagnostic/checker` + DbIndex + old SemanticModel) is disabled;
//! see `migration notes` §M2 for the decommission list.

use std::sync::Arc;

use crate::check::CheckConfig;
use crate::{DiagnosticCode, EmmyLuaAnalysis, Emmyrc, FileId};
use lsp_types::Diagnostic;
use tokio_util::sync::CancellationToken;

/// Separates a completed diagnostic pass from an ineligible file or interrupted work.
#[derive(Debug)]
pub enum FileDiagnosticResult {
    /// A complete report, including an empty report that clears previous diagnostics.
    Complete(Vec<Diagnostic>),
    /// Diagnostics are disabled or the file has no current source text.
    Skipped,
    /// The request or analysis lifetime was cancelled before the report completed.
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct LuaDiagnostic {
    enable: bool,
    check_config: Arc<CheckConfig>,
}

impl Default for LuaDiagnostic {
    fn default() -> Self {
        Self::new()
    }
}

impl LuaDiagnostic {
    pub fn new() -> Self {
        Self {
            enable: true,
            check_config: Arc::new(CheckConfig::default()),
        }
    }

    pub fn update_config(&mut self, emmyrc: Arc<Emmyrc>) {
        self.enable = emmyrc.diagnostics.enable;
        self.check_config = CheckConfig::new(&emmyrc).into();
    }

    // Enable only the specified diagnostic
    pub fn enable_only(&mut self, code: DiagnosticCode) {
        let mut emmyrc = Emmyrc::default();
        emmyrc.diagnostics.enables.push(code);
        for diagnostic_code in DiagnosticCode::all().iter() {
            if *diagnostic_code != code {
                emmyrc.diagnostics.disable.push(*diagnostic_code);
            }
        }
        self.check_config = CheckConfig::new(&emmyrc).into();
    }

    pub fn diagnose_file(
        &self,
        analysis: &EmmyLuaAnalysis,
        file_id: FileId,
        cancel_token: CancellationToken,
    ) -> Option<Vec<Diagnostic>> {
        match self.diagnose_file_result(analysis, file_id, cancel_token) {
            FileDiagnosticResult::Complete(diagnostics) => Some(diagnostics),
            FileDiagnosticResult::Skipped | FileDiagnosticResult::Cancelled => None,
        }
    }

    /// Reports why a file produced no diagnostics, so hosts retry only interrupted work.
    pub fn diagnose_file_result(
        &self,
        analysis: &EmmyLuaAnalysis,
        file_id: FileId,
        cancel_token: CancellationToken,
    ) -> FileDiagnosticResult {
        // Ineligible files remain skipped even when a previous lifetime is cancelled.
        if !self.enable
            || analysis.db.get_file_text(file_id).is_none()
            || analysis.db.line_index(file_id).is_none()
        {
            return FileDiagnosticResult::Skipped;
        }
        let lifetime = analysis.diagnostic_cancellation_token();
        match analysis.diagnose_file_cancellable(
            file_id,
            self.check_config.clone(),
            cancel_token.clone(),
        ) {
            Some(diagnostics) => FileDiagnosticResult::Complete(diagnostics),
            None if cancel_token.is_cancelled() || lifetime.is_cancelled() => {
                FileDiagnosticResult::Cancelled
            }
            None => FileDiagnosticResult::Skipped,
        }
    }
}
