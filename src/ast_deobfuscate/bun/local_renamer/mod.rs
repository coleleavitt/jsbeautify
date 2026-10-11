//! Scope-aware local slot renamer for Bun-minified bundles.
//!
//! Bun mangles function-local bindings to single characters (`H`, `$`, `q`,
//! `K`, `_`, …) that the export/class/param recovery passes leave untouched —
//! the dominant residual in a deobfuscated bundle (~hundreds of thousands of
//! tokens). This pass renames those single-char locals to readable per-scope
//! slot names (`v0`, `v1`, …).
//!
//! ## Correctness
//!
//! Renaming is keyed by [`SymbolId`], so every reference follows its binding
//! automatically (distinct bindings that happen to share the mangled name `H`
//! are distinct symbols and are renamed independently). To avoid introducing a
//! collision or wrongful shadow — which `node --check` cannot detect because it
//! is a *semantic* error, not a syntax one — each scope's slot names are chosen
//! to avoid every name that could capture a rewritten reference: its own
//! bindings, descendant bindings, all ancestor bindings, and the slot names
//! already handed to ancestor scopes. Scopes are processed outer-to-inner so
//! ancestor assignments are known first.
//!
//! Bun names carry no original-name information, so this is *stabilisation*
//! (distinct, greppable, diff-stable names), not semantic recovery.

use oxc_traverse::TraverseCtx;
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::state::DeobfuscateState;
use oxc_syntax::symbol::SymbolId;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;
pub type LocalRenameMap = FxHashMap<SymbolId, String>;

mod names;
mod planner;
mod renamer;
mod types;

pub use names::{is_likely_generated_alias, is_meaningful_short, is_usable_base};
pub use planner::{plan_local_renames, plan_local_renames_with_stats};
pub use renamer::LocalRenamer;
pub use types::{LocalRenamePlanStats, RenameMode};

#[cfg(test)]
mod tests;
