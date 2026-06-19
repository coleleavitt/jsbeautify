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
        expr.clone_in_with_semantic_ids(ctx.ast.allocator)
    }

    pub(super) fn make_number<'a>(val: i64, ctx: &mut Ctx<'a>) -> Expression<'a> {
        Expression::NumericLiteral(ctx.ast.alloc(NumericLiteral {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            value: val as f64,
            raw: Some(ctx.ast.str(&val.to_string())),
            base: NumberBase::Decimal,
        }))
    }

    pub(super) fn make_boolean<'a>(val: bool, ctx: &mut Ctx<'a>) -> Expression<'a> {
        Expression::BooleanLiteral(ctx.ast.alloc(BooleanLiteral {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            value: val,
        }))
    }
}
