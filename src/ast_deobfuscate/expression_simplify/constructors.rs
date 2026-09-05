use oxc_allocator::CloneIn;
use oxc_ast::ast::Expression;
use oxc_span::SPAN;
use oxc_syntax::number::NumberBase;

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
        expr.clone_in_with_semantic_ids(ctx)
    }

    pub(super) fn make_boolean<'a>(val: bool, ctx: &mut Ctx<'a>) -> Expression<'a> {
        Expression::new_boolean_literal(SPAN, val, ctx)
    }

    pub(super) fn make_number<'a>(val: i64, ctx: &mut Ctx<'a>) -> Expression<'a> {
        Expression::new_numeric_literal(
            SPAN,
            val as f64,
            Some(oxc_ast::ast::Str::from_str_in(&val.to_string(), ctx)),
            NumberBase::Decimal,
            ctx,
        )
    }
}
