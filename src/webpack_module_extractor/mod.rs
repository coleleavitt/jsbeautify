#[cfg(debug_assertions)]
macro_rules! trace_webpack {
    ($($arg:tt)*) => {
        eprintln!("[WEBPACK] {}", format!($($arg)*));
    };
}

#[cfg(not(debug_assertions))]
macro_rules! trace_webpack {
    ($($arg:tt)*) => {};
}

mod deps;
mod legacy_jsonp;
mod output;
mod parser;
mod types;

use crate::Result;
use crate::token::Token;
use std::collections::HashMap;

pub use types::WebpackModule;

pub struct ModuleExtractor {
    modules: HashMap<usize, WebpackModule>,
}

impl Default for ModuleExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl ModuleExtractor {
    #[must_use]
    pub fn new() -> Self {
        trace_webpack!("initializing ModuleExtractor");
        Self {
            modules: HashMap::new(),
        }
    }

    /// # Errors
    /// Returns an error if token scanning overflows while locating a module map.
    pub fn extract_modules(&mut self, tokens: &[Token]) -> Result<()> {
        trace_webpack!("=== EXTRACTING WEBPACK MODULES ===");
        trace_webpack!("total tokens: {}", tokens.len());

        self.find_module_map(tokens)?;

        trace_webpack!("found {} modules", self.modules.len());
        Ok(())
    }

    #[must_use]
    pub fn module_count(&self) -> usize {
        self.modules.len()
    }
}
