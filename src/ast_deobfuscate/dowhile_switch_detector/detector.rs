use oxc_ast::ast::Program;

use super::types::DoWhileDispatcherMap;
use super::walker::walk_stmts;

#[derive(Debug, Default)]
pub struct DoWhileSwitchDetector {
    dispatchers: DoWhileDispatcherMap,
}

impl DoWhileSwitchDetector {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn detect(mut self, program: &Program<'_>) -> DoWhileDispatcherMap {
        dwsd_dbg!(
            "[DBG/dwsd] detect: starting walk of {} top-level statements",
            program.body.len()
        );
        walk_stmts(&program.body, &mut self.dispatchers);
        dwsd_dbg!(
            "[DBG/dwsd] detect: finished, found {} dispatchers",
            self.dispatchers.len()
        );
        self.dispatchers
    }
}
