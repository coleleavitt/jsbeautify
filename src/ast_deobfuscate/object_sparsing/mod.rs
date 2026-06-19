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
    changed: bool,
}

impl ObjectSparsingConsolidator {
    #[must_use]
    pub const fn new() -> Self {
        Self { changed: false }
    }
    #[must_use]
    pub const fn has_changed(&self) -> bool {
        self.changed
    }
}
impl Default for ObjectSparsingConsolidator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
