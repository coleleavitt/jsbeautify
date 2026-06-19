use oxc_ast::ast::Program;

use super::types::DispatcherMap;
use super::walker::walk_stmts;

/// Read-only scanner that detects CFF dispatcher functions.
#[derive(Debug, Default)]
pub struct DispatcherDetector {
    dispatchers: DispatcherMap,
}

impl DispatcherDetector {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Scan the program and return all detected dispatchers.
    #[must_use]
    pub fn detect(mut self, program: &Program<'_>) -> DispatcherMap {
        cff_dbg!(
            "[DBG/disp] detect: starting walk of {} top-level statements",
            program.body.len()
        );
        walk_stmts(&program.body, &mut self.dispatchers);
        cff_dbg!(
            "[DBG/disp] detect: finished, found {} dispatchers",
            self.dispatchers.len()
        );
        self.dispatchers
    }
}
