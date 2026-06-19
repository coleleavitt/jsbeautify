//! Trampoline inliner for state-machine dispatchers.
//!
//! Akamai BMP (and similar Jscrambler-based obfuscators) expand every logical
//! function into a tiny trampoline into a central dispatcher:
//!
//! ```text
//! var xB = function() { return LT.apply(this, [S, arguments]); };
//! function jQ()         { return JA.apply(this, [Qz, arguments]); }
//! ```
//!
//! where `LT` / `JA` / `hY` / `SY` / `ZE` / `tQ` / … are top-level interpreter
//! functions that dispatch on their first argument (a state constant such as
//! `S`, `Qz`, `ll`, `E5`, …). Hundreds of call sites `xB(a, b, c)` are
//! semantically equivalent to `LT.call(this, S, [a, b, c])`.
//!
//! This pass:
//! 1. Collects trampolines. Recognised shapes:
//!    * `function NAME() { return DISPATCH.apply(this, [STATE, arguments]); }`
//!    * `var NAME = function() { return DISPATCH.apply(this, [STATE, arguments]); };`
//!    * `var NAME = function NAME2() { return DISPATCH.apply(this, [STATE, arguments]); };`
//!    * `NAME = function() { return DISPATCH.apply(this, [STATE, arguments]); };`
//!      `DISPATCH` must be a plain identifier; `STATE` must be a plain identifier.
//! 2. At every call site `NAME(<args>)` (identifier call, not member call), rewrites to
//!    `DISPATCH.call(this, STATE, [<args>])`.
//! 3. Removes the trampoline declaration.
//!
//! The `this` is preserved by using `.call(this, …)`. The `arguments` object
//! becomes an explicit array-literal argument — the dispatchers already
//! index into their 2nd parameter as `arg[idx]`, so array-literal works.

use oxc_traverse::TraverseCtx;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

mod collector;
mod inliner;
mod types;

pub use collector::TrampolineCollector;
pub use inliner::TrampolineInliner;
pub use types::TrampolineInfo;

#[cfg(test)]
mod tests;
