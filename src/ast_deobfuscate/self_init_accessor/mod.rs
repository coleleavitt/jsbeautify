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
//! with `var __hx_cache = <init>;` inserted at the start of the function body
//! or program that declares `hx`, so each run of that body gets its own cache,
//! as each run creates its own `hx`. The rewrite preserves the cached-singleton
//! semantics — every call to `hx()` returns the same object — while removing
//! the self-reassignment that makes the code unreadable. The cache name is
//! one the program does not use.
//!
//! `hx`, `x` and the builtins are resolved by symbol. We only rewrite accessors
//! whose initialiser is one of a whitelist of pure, side-effect-free expressions
//! (`[]`, `{}`, `new Object()`, `new Array()`, `Object.create(null)`,
//! `Object.create({})`, `[].entries()` / `[].keys()` / `[].values()`, with
//! `Object` and `Array` the globals), and whose every other reference is a
//! plain call `hx()`: code that kept the original function would initialise a
//! new object on each call. Async and generator functions, bindings a `with`
//! object, an Annex B function or a direct `eval` may replace, and
//! classic-script globals are left alone. Built-in prototypes are assumed
//! unpatched.

use oxc_traverse::TraverseCtx;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

mod detect;
mod flattener;
mod types;

pub use flattener::SelfInitAccessorFlattener;

#[cfg(test)]
mod tests;
