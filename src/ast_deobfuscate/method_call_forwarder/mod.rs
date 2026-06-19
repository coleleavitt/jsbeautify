//! 2-arg method-call forwarder inliner.
//!
//! Detects wrappers that forward their two parameters to a fixed method call
//! on the first argument:
//!
//! ```text
//! function J(a, b) { return a.charCodeAt(b); }
//! var J = function(a, b) { return a.charCodeAt(b); };
//! ```
//!
//! Every call site `J(x, y)` becomes `x.charCodeAt(y)` and the wrapper
//! declaration is removed. The pass only fires when:
//! - exactly 2 params
//! - body is a single `return <param0>.<METHOD>(<param1>);`
//! - method name is a plain identifier (no computed member)
//! - no spread in either argument list
//!
//! This complements [`LookupForwarderInliner`] (which handles
//! `function F(x) { return acc()[x]; }`) by covering the other common
//! BMP/Jscrambler wrapper shape.

use oxc_traverse::TraverseCtx;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

mod collector;
mod inliner;
mod types;

pub use collector::MethodCallForwarderCollector;
pub use inliner::MethodCallForwarderInliner;
pub use types::MethodForwarderInfo;

#[cfg(test)]
mod tests;
