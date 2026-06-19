//! Class renamer based on `this.name = "ClassName"` patterns.
//!
//! Bun-minified bundles often have patterns like:
//! ```js
//! c5 = class c5 extends Error {
//!     constructor(H) {
//!         super(H);
//!         this.name = "AbortError";
//!     }
//! };
//! ```
//! This pass detects `this.name = "X"` inside class constructors and
//! renames the class from its mangled name to the real name.
//!
//! Also handles `.displayName = "ComponentName"` for React components.

use oxc_syntax::symbol::SymbolId;
use oxc_traverse::TraverseCtx;
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;
pub type ClassNameMap = FxHashMap<SymbolId, String>;

mod collectors;
mod renamer;
mod util;

pub use collectors::{ClassNameCollector, DisplayNameCollector};
pub use renamer::ClassNameRenamer;

#[cfg(test)]
mod tests;
