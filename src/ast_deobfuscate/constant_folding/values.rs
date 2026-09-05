use oxc_allocator::GetAllocator;
use oxc_ast::ast::{Expression, UnaryOperator};
use oxc_span::SPAN;
use oxc_syntax::number::NumberBase;

use super::{ConstantFolder, Ctx};

impl ConstantFolder {
    pub(super) fn extract_number(expr: &Expression<'_>) -> Option<i64> {
        match expr {
            Expression::NumericLiteral(lit) => {
                let val = lit.value;
                if val.fract() != 0.0 || val > i64::MAX as f64 || val < i64::MIN as f64 {
                    return None;
                }
                Some(val as i64)
            }
            Expression::UnaryExpression(unary) if unary.operator == UnaryOperator::UnaryNegation => {
                let inner = Self::extract_number(&unary.argument)?;
                inner.checked_neg()
            }
            _ => None,
        }
    }

    pub(super) fn extract_boolean(expr: &Expression<'_>) -> Option<bool> {
        match expr {
            Expression::BooleanLiteral(lit) => Some(lit.value),
            _ => None,
        }
    }

    pub(super) fn make_number<'a>(val: i64, ctx: &mut Ctx<'a>) -> Expression<'a> {
        let raw = Some(oxc_allocator::FromIn::from_in(val.to_string(), ctx.allocator()));
        Expression::new_numeric_literal(SPAN, val as f64, raw, NumberBase::Decimal, ctx)
    }

    pub(super) fn make_boolean<'a>(val: bool, ctx: &mut Ctx<'a>) -> Expression<'a> {
        Expression::new_boolean_literal(SPAN, val, ctx)
    }
}
