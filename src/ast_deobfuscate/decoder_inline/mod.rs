//! Decoder function detection and inlining pass
//!
//! Detects decoder functions that wrap string array access:
//! ```js
//! var _0x1234 = ["hello", "world"];
//! function _0xdec(a) { return _0x1234[a - 291]; }
//! console.log(_0xdec(291)); // Inlined to: console.log("hello");
//! ```

use oxc_ast::ast::{Expression, Program, Statement};
use oxc_syntax::scope::ScopeId;
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::state::{DecoderInfo, DeobfuscateState};
use crate::ast_deobfuscate::with_scope::scopes_in_with_bodies;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

mod access;
mod decoding;
mod detection;
mod inline;

pub struct DecoderInliner {
    detected_decoders: FxHashMap<String, DecoderInfo>,
    /// Calls here may reach a `with` object's property instead of the decoder.
    with_body_scopes: FxHashSet<ScopeId>,
    rewrites: usize,
}

impl DecoderInliner {
    #[must_use]
    pub fn new() -> Self {
        Self {
            detected_decoders: FxHashMap::default(),
            with_body_scopes: FxHashSet::default(),
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
    fn enter_program(&mut self, _program: &mut Program<'a>, ctx: &mut Ctx<'a>) {
        self.with_body_scopes = scopes_in_with_bodies(ctx.scoping());
    }

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
            && !self.with_body_scopes.contains(&ctx.current_scope_id())
            && let Some(new_expr) = self.try_inline_decoder_call(call, ctx)
        {
            *expr = new_expr;
            self.rewrites += 1;
        }
    }
}

#[cfg(test)]
mod tests;
