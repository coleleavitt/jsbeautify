use oxc_allocator::{CloneIn, GetAllocator};
use oxc_ast::ast::{Expression, LogicalExpression, LogicalOperator};
use oxc_span::SPAN;

use super::{AlgebraicSimplifier, Ctx};

/// Only a literal left operand is folded: `x && true` evaluates to `x` (not
/// `true`) and `f() || true` must still call `f`, so a literal right operand
/// says nothing about the result.
impl AlgebraicSimplifier {
    pub(super) fn try_simplify_logical_and<'a>(
        &mut self,
        logical: &LogicalExpression<'a>,
        ctx: &mut Ctx<'a>,
    ) -> Option<Expression<'a>> {
        if logical.operator != LogicalOperator::And {
            return None;
        }
        let Expression::BooleanLiteral(left) = &logical.left else {
            return None;
        };
        self.changed = true;
        if left.value {
            eprintln!("[AST] Simplifying true && x to x");
            Some(logical.right.clone_in_with_semantic_ids(ctx.ast.allocator()))
        } else {
            eprintln!("[AST] Simplifying false && x to false");
            Some(Expression::new_boolean_literal(SPAN, false, ctx))
        }
    }

    pub(super) fn try_simplify_logical_or<'a>(
        &mut self,
        logical: &LogicalExpression<'a>,
        ctx: &mut Ctx<'a>,
    ) -> Option<Expression<'a>> {
        if logical.operator != LogicalOperator::Or {
            return None;
        }
        let Expression::BooleanLiteral(left) = &logical.left else {
            return None;
        };
        self.changed = true;
        if left.value {
            eprintln!("[AST] Simplifying true || x to true");
            Some(Expression::new_boolean_literal(SPAN, true, ctx))
        } else {
            eprintln!("[AST] Simplifying false || x to x");
            Some(logical.right.clone_in_with_semantic_ids(ctx.ast.allocator()))
        }
    }
}
