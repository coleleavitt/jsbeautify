//! Window-alias propagator: `var Q6 = window; Q6.document` → `document`.
//!
//! Detects variables assigned only to global-like values (`window`, `globalThis`,
//! `global`, `self`, `this`) and replaces `alias.prop` with just `prop`.
//!
//! Aliases are tracked by resolved [`SymbolId`], NOT by raw name. A minified
//! bundle routinely reuses the same short name (`props`, `e`, `t`) for unrelated
//! bindings in different scopes; one `var props = this` must not cause every
//! other function's `props.x` to be stripped. (It also guards against rewriting
//! `alias.in` → `in`, which would emit a reserved word and produce unparseable
//! output.)

use crate::ast_deobfuscate::state::DeobfuscateState;
use oxc_traverse::TraverseCtx;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

mod collector;
mod propagator;
mod utils;

pub use collector::WindowAliasCollector;
pub use propagator::WindowAliasPropagator;

#[cfg(test)]
mod tests;
