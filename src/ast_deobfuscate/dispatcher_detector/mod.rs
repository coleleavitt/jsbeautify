//! CFF (Control-Flow-Flattening) dispatcher detector.
//!
//! Scans a JavaScript AST for dispatcher functions — functions with exactly
//! 2 parameters whose body is a single `switch` statement on the first
//! parameter, with `case IDENT: { ... } break;` clauses.
//!
//! This is a read-only scan (no AST mutation). It builds a [`DispatcherMap`]
//! that maps dispatcher names to their [`DispatcherInfo`] metadata.

fn cff_dbg_enabled() -> bool {
    std::env::var("AST_CFF_DEBUG").is_ok()
}

macro_rules! cff_dbg {
    ($($arg:tt)*) => {
        if crate::ast_deobfuscate::dispatcher_detector::cff_dbg_enabled() { eprintln!($($arg)*); }
    };
}

mod detector;
mod extraction;
mod types;
mod walker;

pub use detector::DispatcherDetector;
pub use types::{CaseInfo, DispatcherInfo, DispatcherMap};

#[cfg(test)]
mod tests;
