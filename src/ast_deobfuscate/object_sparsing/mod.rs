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

use oxc_traverse::TraverseCtx;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

mod consolidate;
mod types;

pub struct ObjectSparsingConsolidator {
    rewrites: usize,
}

impl ObjectSparsingConsolidator {
    #[must_use]
    pub const fn new() -> Self {
        Self { rewrites: 0 }
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
