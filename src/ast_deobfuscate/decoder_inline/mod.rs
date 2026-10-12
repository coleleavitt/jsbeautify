//! Decoder function detection and inlining pass
//!
//! Detects decoder functions that wrap string array access:
//! ```js
//! var _0x1234 = ["hello", "world"];
//! function _0xdec(a) { return _0x1234[a - 291]; }
//! console.log(_0xdec(291)); // Inlined to: console.log("hello");
//! ```

use oxc_ast::ast::{Expression, Statement};
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::state::{DecoderInfo, DeobfuscateState};

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

mod access;
mod decoding;
mod detection;
mod inline;

pub struct DecoderInliner {
    detected_decoders: FxHashMap<String, DecoderInfo>,
    rewrites: usize,
}

impl DecoderInliner {
    #[must_use]
    pub fn new() -> Self {
        Self {
            detected_decoders: FxHashMap::default(),
            rewrites: 0,
        }
    }

    #[must_use]
    pub const fn rewrites(&self) -> usize {
        self.rewrites
    }

    pub fn finalize(&mut self, state: &mut DeobfuscateState) {
        state.decoders.extend(self.detected_decoders.drain());
    }
}

impl Default for DecoderInliner {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for DecoderInliner {
    fn enter_statement(&mut self, stmt: &mut Statement<'a>, ctx: &mut Ctx<'a>) {
        if let Statement::FunctionDeclaration(func_decl) = stmt {
            let func_name = func_decl.id.as_ref().map_or("", |id| id.name.as_str());

            if !func_name.is_empty()
                && let Some(decoder) = self.detect_decoder_function(func_decl, func_name, ctx)
            {
                deob_trace!(
                    "[AST] Storing decoder: {} -> array={}, offset={}, op={:?}",
                    decoder.function_name,
                    decoder.array_name,
                    decoder.offset,
                    decoder.offset_operation
                );
                self.detected_decoders.insert(decoder.function_name.clone(), decoder);
            }
        }
    }

    fn enter_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        if let Expression::CallExpression(call) = expr
            && let Some(new_expr) = self.try_inline_decoder_call(call, ctx)
        {
            *expr = new_expr;
            self.rewrites += 1;
        }
    }
}

#[cfg(test)]
mod tests;
