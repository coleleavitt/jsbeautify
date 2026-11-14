use oxc_ast::ast::{Expression, Program, Str};
use oxc_span::SPAN;
use oxc_syntax::scope::ScopeId;
use oxc_traverse::Traverse;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::state::DeobfuscateState;
use crate::ast_deobfuscate::with_scope::scopes_in_with_bodies;

use super::Ctx;
use super::eval::try_eval_as_usize;

pub struct DispatchInlinerRewriter {
    factories: FxHashMap<String, Vec<String>>,
    constants: FxHashMap<String, usize>,
    /// Calls here may reach a `with` object's property instead of the factory.
    with_body_scopes: FxHashSet<ScopeId>,
    inlined: usize,
}

impl DispatchInlinerRewriter {
    #[must_use]
    pub fn new(factories: FxHashMap<String, Vec<String>>, constants: FxHashMap<String, usize>) -> Self {
        Self {
            factories,
            constants,
            with_body_scopes: FxHashSet::default(),
            inlined: 0,
        }
    }

    #[must_use]
    pub const fn inlined(&self) -> usize {
        self.inlined
    }

    fn resolve_index(&self, expr: &Expression<'_>) -> Option<usize> {
        try_eval_as_usize(expr, &self.constants)
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for DispatchInlinerRewriter {
    fn enter_program(&mut self, _program: &mut Program<'a>, ctx: &mut Ctx<'a>) {
        self.with_body_scopes = scopes_in_with_bodies(ctx.scoping());
    }

    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        let Expression::ComputedMemberExpression(cme) = expr else {
            return;
        };
        if self.with_body_scopes.contains(&ctx.current_scope_id()) {
            return;
        }
        let Expression::CallExpression(call) = &cme.object else {
            return;
        };
        if !call.arguments.is_empty() {
            return;
        }
        let Expression::Identifier(callee) = &call.callee else {
            return;
        };
        let Some(strings) = self.factories.get(callee.name.as_str()) else {
            return;
        };
        let Some(index) = self.resolve_index(&cme.expression) else {
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
