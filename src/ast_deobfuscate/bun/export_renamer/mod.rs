//! Bun __`export()` name extraction and renaming pass
//!
//! Extracts original export names from `__export(target, { originalName: () => mangledName })`
//! calls and renames `mangledName` → `originalName` throughout the program.
//!
//! This is the AST-based replacement for the regex approach in `bun_extract.rs`.
//! It fires during the deobfuscation pipeline (Phase 0.95) so that all subsequent
//! passes see the readable names. The minified local behind a `default` key, or
//! behind a native `export default` / `export { x as default }`, is named by
//! [`crate::ast_deobfuscate::default_export`].
//!
//! Example:
//! ```js
//! __export(GZ8, {
//!     writeToStdout: () => UK,
//!     exitWithError: () => Ba4
//! });
//! function UK(data) { process.stdout.write(data); }
//! ```
//! becomes:
//! ```js
//! __export(GZ8, {
//!     writeToStdout: () => writeToStdout,
//!     exitWithError: () => exitWithError
//! });
//! function writeToStdout(data) { process.stdout.write(data); }
//! ```

use oxc_traverse::TraverseCtx;
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::state::DeobfuscateState;
use oxc_syntax::symbol::SymbolId;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;
pub type ExportNameMap = FxHashMap<SymbolId, String>;

mod collector;
pub(crate) mod names;
mod renamer;
mod unique;

pub use collector::ExportNameCollector;
pub use renamer::ExportNameRenamer;
pub use unique::make_names_unique;

#[cfg(test)]
mod tests;
