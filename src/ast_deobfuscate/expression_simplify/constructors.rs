use oxc_allocator::CloneIn;
use oxc_ast::ast::{BooleanLiteral, Expression, NumericLiteral};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use oxc_syntax::number::NumberBase;
use std::cell::Cell;

use super::{Ctx, ExpressionSimplifier};

impl ExpressionSimplifier {
    pub(super) fn is_valid_identifier(s: &str) -> bool {
        if s.is_empty() {
            return false;
        }

        let mut chars = s.chars();
        let first = chars.next().expect("non-empty after is_empty check");

        if !first.is_alphabetic() && first != '_' && first != '$' {
            return false;
        }

        chars.all(|c| c.is_alphanumeric() || c == '_' || c == '$')
    }

    pub(super) fn clone_expression<'a>(expr: &Expression<'a>, ctx: &mut Ctx<'a>) -> Expression<'a> {
        expr.clone_in_with_semantic_ids(ctx.ast.allocator)
    }

    pub(super) fn make_boolean<'a>(val: bool, ctx: &mut Ctx<'a>) -> Expression<'a> {
        Expression::BooleanLiteral(ctx.ast.alloc(BooleanLiteral {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            value: val,
        }))
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
}
