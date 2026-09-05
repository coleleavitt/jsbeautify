use oxc_allocator::{CloneIn, GetAllocator};
use oxc_ast::ast::{Expression, Statement};

use super::{CommonSubexpressionElimination, Ctx};

impl CommonSubexpressionElimination {
    pub(super) fn clone_statement<'a>(&self, stmt: &Statement<'a>, ctx: &mut Ctx<'a>) -> Statement<'a> {
        stmt.clone_in(ctx.ast.allocator())
    }

    #[allow(dead_code)]
    pub(super) fn clone_expression<'a>(expr: &Expression<'a>, ctx: &mut Ctx<'a>) -> Expression<'a> {
        expr.clone_in(ctx.ast.allocator())
    }
}
