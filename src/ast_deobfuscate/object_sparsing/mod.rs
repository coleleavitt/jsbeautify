//! Object Sparsing Consolidation
//!
//! Transforms sparse object construction patterns:
//! ```js
//! var obj = {};
//! obj.a = 1;
//! obj.b = 2;
//! ```
//! Into:
//! ```js
//! var obj = {a: 1, b: 2};
//! ```
//!
//! The object and the assignments are matched by `SymbolId`. A value is moved
//! into the literal only when reading it earlier cannot be observed: a
//! literal, a `var`, parameter or function other than the object itself, or
//! the global `undefined`, `NaN` or `Infinity`. Assigning a property is assumed
//! not to hit a setter on `Object.prototype`.

use oxc_span::SourceType;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::TraverseCtx;
use rustc_hash::FxHashSet;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

mod consolidate;
mod types;

pub struct ObjectSparsingConsolidator {
    rewrites: usize,
    /// Bindings a `with` object, an Annex B function or a direct `eval` may
    /// stand in for.
    redirectable: FxHashSet<SymbolId>,
    source_type: SourceType,
}

impl ObjectSparsingConsolidator {
    #[must_use]
    pub fn new() -> Self {
        Self {
            rewrites: 0,
            redirectable: FxHashSet::default(),
            source_type: SourceType::default(),
        }
    }
    #[must_use]
    pub const fn rewrites(&self) -> usize {
        self.rewrites
    }
}
impl Default for ObjectSparsingConsolidator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
