//! String array detection and rotation pass
//!
//! Detects obfuscated string arrays and rotation IIFEs:
//! ```js
//! var _0x1234 = ["a", "b", "c"];
//! (function(_0x5678, _0x9abc) {
//!     var _0xdef = function(_0xghi) {
//!         while (--_0xghi) {
//!             _0x5678.push(_0x5678.shift());
//!         }
//!     };
//!     _0xdef(2);
//! })(_0x1234, 0x123);
//! ```
//! The array is rotated, so `["a", "b", "c"]` becomes `["c", "a", "b"]` after 2 rotations.

use oxc_ast::ast::{Expression, Statement};
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::state::{DeobfuscateState, StringArrayInfo};

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

mod detection;
mod rotation;

pub struct StringArrayRotation {
    detected_arrays: FxHashMap<String, StringArrayInfo>,
    pending_rotations: FxHashMap<String, usize>,
}

impl StringArrayRotation {
    #[must_use]
    pub fn new() -> Self {
        Self {
            detected_arrays: FxHashMap::default(),
            pending_rotations: FxHashMap::default(),
        }
    }

    /// `(string arrays, rotation IIFEs)` detected so far.
    #[must_use]
    pub fn detected(&self) -> (usize, usize) {
        (self.detected_arrays.len(), self.pending_rotations.len())
    }

    pub fn finalize(&mut self, state: &mut DeobfuscateState) {
        for (array_name, rotation_count) in &self.pending_rotations {
            if let Some(array_info) = self.detected_arrays.get_mut(array_name) {
                Self::apply_rotation(&mut array_info.strings, *rotation_count);
                array_info.rotated = true;
                array_info.rotation_count = *rotation_count;
                deob_trace!("[AST] Applied rotation to {array_name}: {rotation_count} positions");
                deob_trace!(
                    "[AST]   Result: {:?}",
                    &array_info.strings[..5.min(array_info.strings.len())]
                );
            }
        }
        state.string_arrays.extend(self.detected_arrays.drain());
    }
}

impl Default for StringArrayRotation {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for StringArrayRotation {
    fn enter_statement(&mut self, stmt: &mut Statement<'a>, _ctx: &mut Ctx<'a>) {
        if let Statement::VariableDeclaration(var_decl) = stmt
            && let Some(array_info) = self.detect_string_array(var_decl)
        {
            deob_trace!(
                "[AST] Found string array: {} with {} strings",
                array_info.var_name,
                array_info.strings.len()
            );
            deob_trace!(
                "[AST]   First 5: {:?}",
                &array_info.strings[..5.min(array_info.strings.len())]
            );
            self.detected_arrays.insert(array_info.var_name.clone(), array_info);
        }

        if let Statement::ExpressionStatement(expr_stmt) = stmt
            && let Expression::CallExpression(call) = &expr_stmt.expression
            && let Some((array_name, rotation_count)) = self.detect_rotation_iife(call)
        {
            deob_trace!("[AST] Detected rotation IIFE for {array_name}: {rotation_count} rotations");
            self.pending_rotations.insert(array_name, rotation_count);
        }
    }
}

#[cfg(test)]
mod tests;
