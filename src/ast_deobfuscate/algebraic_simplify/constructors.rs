use oxc_allocator::CloneIn;
use oxc_ast::ast::{BooleanLiteral, Expression, NumericLiteral};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use oxc_syntax::number::NumberBase;
use std::cell::Cell;

use super::{AlgebraicSimplifier, Ctx};

impl AlgebraicSimplifier {
    pub(super) fn is_zero(expr: &Expression<'_>) -> bool {
        if let Expression::NumericLiteral(num) = expr {
            num.value == 0.0
        } else {
            false
        }
    }

    pub(super) fn is_one(expr: &Expression<'_>) -> bool {
        if let Expression::NumericLiteral(num) = expr {
            num.value == 1.0
        } else {
            false
        }
    }

    pub(super) fn is_true(expr: &Expression<'_>) -> bool {
        matches!(expr, Expression::BooleanLiteral(b) if b.value)
    }

    pub(super) fn is_false(expr: &Expression<'_>) -> bool {
        matches!(expr, Expression::BooleanLiteral(b) if !b.value)
    }

    pub(super) fn clone_expression<'a>(expr: &Expression<'a>, ctx: &mut Ctx<'a>) -> Expression<'a> {
        expr.clone_in_with_semantic_ids(ctx.allocator())
    }

    pub(super) fn make_number<'a>(val: i64, ctx: &mut Ctx<'a>) -> Expression<'a> {
        Expression::new_numeric_literal(
            SPAN,
            val as f64,
            Some(oxc_ast::ast::Str::new_in(&val.to_string(), ctx)),
            NumberBase::Decimal,
            ctx,
        )
    }

    pub(super) fn make_boolean<'a>(val: bool, ctx: &mut Ctx<'a>) -> Expression<'a> {
        Expression::new_boolean_literal(SPAN, val, ctx)
    }
}
