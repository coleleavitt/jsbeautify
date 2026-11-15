//! String-array-factory dispatch inliner.
//!
//! Resolves `x19()[CONST]` and `Y49()[CONST]` patterns into literal strings.
//!
//! Detects function declarations that return a literal string array:
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
//! Also resolves index constants: `var CONST = 1 + OTHER;` or `CONST = 2;`.
//!
//! Factories, constants and call sites are matched by `SymbolId` (an
//! undeclared global by its unresolved references), never by name. A
//! binding qualifies only when nothing but its own definition can give it a
//! value: see [`DispatchInlinerCollector`].

use oxc_syntax::symbol::SymbolId;
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::state::DeobfuscateState;
use oxc_traverse::TraverseCtx;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

mod collector;
mod eval;
mod rewriter;

pub use collector::DispatchInlinerCollector;
pub use rewriter::DispatchInlinerRewriter;

/// A binding an index constant is stored in.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Binding {
    Symbol(SymbolId),
    /// An undeclared global, which every unresolved reference of this name reads.
    Global(String),
}

struct Constant {
    value: usize,
    /// The end of the definition: a read from this offset on sees the value.
    ready: u32,
}

/// The index constants the collector proved.
#[derive(Default)]
pub struct IndexConstants(FxHashMap<Binding, Constant>);

impl IndexConstants {
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// The value a read of `binding` at offset `at` sees.
    fn get(&self, binding: &Binding, at: u32) -> Option<usize> {
        self.0
            .get(binding)
            .filter(|constant| at >= constant.ready)
            .map(|constant| constant.value)
    }
}

#[cfg(test)]
mod tests;
