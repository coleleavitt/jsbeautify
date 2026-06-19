use oxc_ast::ast::{Expression, StringLiteral};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use oxc_traverse::Traverse;
use rustc_hash::FxHashMap;
use std::cell::Cell;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;
use super::eval::try_eval_as_usize;

pub struct DispatchInlinerRewriter {
    factories: FxHashMap<String, Vec<String>>,
    constants: FxHashMap<String, usize>,
    inlined: usize,
}

impl DispatchInlinerRewriter {
    #[must_use]
    pub const fn new(factories: FxHashMap<String, Vec<String>>, constants: FxHashMap<String, usize>) -> Self {
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

    fn resolve_index(&self, expr: &Expression<'_>) -> Option<usize> {
        try_eval_as_usize(expr, &self.constants)
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
            eprintln!(
                "[AST/dispatch-inline] {}()[{}] -> {:?}",
                callee.name.as_str(),
                index,
                value
            );
        }
        self.inlined += 1;

        *expr = Expression::StringLiteral(ctx.ast.alloc(StringLiteral {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            value: ctx.ast.str(value),
            raw: None,
            lone_surrogates: false,
        }));
    }
}
