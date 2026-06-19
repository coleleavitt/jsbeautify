//! Self-initialising accessor flattener.
//!
//! Rewrites the Akamai-BMP / Jscrambler idiom
//!
//! ```text
//! function hx() {
//!     var x = <init>;           // [] | {} | new Object() | [].entries() | …
//!     hx = function () { return x; };
//!     return x;
//! }
//! ```
//!
//! into the equivalent but far more readable:
//!
//! ```text
//! function hx() { return x; }
//! ```
//!
//! with a hoisted `var x = <init>;` inserted at the start of the enclosing
//! function-scope or program body. The rewrite preserves the cached-singleton
//! semantics — every call to `hx()` returns the same object — while removing
//! the self-reassignment that makes the code unreadable.
//!
//! We only rewrite accessors whose initialiser is one of a whitelist of pure,
//! side-effect-free expressions (`[]`, `{}`, `new Object()`, `new Array()`,
//! `Object.create(...)`, `[].entries()` / `[].keys()` / `[].values()`).
//! Anything else (call to user function, arithmetic, etc.) is left alone.

use oxc_traverse::TraverseCtx;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

mod detect;
mod flattener;
mod types;

pub use flattener::SelfInitAccessorFlattener;

#[cfg(test)]
mod tests;
