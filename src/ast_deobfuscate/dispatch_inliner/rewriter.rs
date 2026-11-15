use oxc_ast::ast::{Expression, Str};
use oxc_span::SPAN;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::Traverse;
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::eval::ConstExpr;
use super::{Ctx, IndexConstants};

/// Replaces `f()[k]` with the string at index `k` when `f` resolves to a
/// factory and `k` to a constant index, as [`super::DispatchInlinerCollector`]
/// proved them.
pub struct DispatchInlinerRewriter {
    factories: FxHashMap<SymbolId, Vec<String>>,
    constants: IndexConstants,
    inlined: usize,
}

impl DispatchInlinerRewriter {
    #[must_use]
    pub const fn new(factories: FxHashMap<SymbolId, Vec<String>>, constants: IndexConstants) -> Self {
        Self {
            factories,
            constants,
            inlined: 0,
        }
    }

    #[must_use]
    pub const fn inlined(&self) -> usize {
        self.inlined
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for DispatchInlinerRewriter {
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        let Expression::ComputedMemberExpression(cme) = expr else {
            return;
        };
        let Expression::CallExpression(call) = &cme.object else {
            return;
        };
        if !call.arguments.is_empty() {
            return;
        }
        let Expression::Identifier(callee) = &call.callee else {
            return;
        };
        let scoping = ctx.scoping();
        let Some(strings) = scoping
            .get_reference(callee.reference_id())
            .symbol_id()
            .and_then(|symbol| self.factories.get(&symbol))
        else {
            return;
        };
        let Some(index) = ConstExpr::from_ast(&cme.expression, scoping)
            .and_then(|index| index.eval(&|binding, at, scope| self.constants.get(binding, at, scope)))
            .and_then(|index| usize::try_from(index).ok())
        else {
            return;
        };
        let Some(value) = strings.get(index) else {
            return;
        };

        if self.inlined < 10 {
            deob_trace!(
                "[AST/dispatch-inline] {}()[{}] -> {:?}",
                callee.name.as_str(),
                index,
                value
            );
        }
        self.inlined += 1;

        *expr = Expression::new_string_literal(SPAN, Str::from_str_in(value.as_str(), ctx), None, ctx);
    }
}
