//! String-array-factory dispatch inliner.
//!
//! Resolves `x19()[CONST]` and `Y49()[CONST]` patterns into literal strings.
//!
//! Detects functions that return a literal string array:
//!
//! ```text
//! function x19() { return ["BR","t5","B5",...]; }
//! ```
//!
//! or the self-init accessor pattern:
//!
//! ```text
//! function x19() {
//!     var vB4 = ["BR","t5","B5",...];
//!     x19 = function() { return vB4; };
//!     return vB4;
//! }
//! ```
//!
//! Then replaces `x19()[5]` with `"BE"` (the string at index 5).
//! Also resolves simple `var CONST = NUMBER;` index constants.

use crate::ast_deobfuscate::state::DeobfuscateState;
use oxc_traverse::TraverseCtx;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

mod collector;
mod eval;
mod rewriter;

pub use collector::DispatchInlinerCollector;
pub use rewriter::DispatchInlinerRewriter;

#[cfg(test)]
mod tests;
