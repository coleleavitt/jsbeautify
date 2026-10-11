use oxc_allocator::GetAllocator;
use oxc_ast::ast::{Expression, UnaryOperator};
use oxc_span::SPAN;
use oxc_syntax::number::NumberBase;

use super::{ConstantFolder, Ctx};

const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

fn is_safe_integer(val: f64) -> bool {
    val.fract() == 0.0 && val.abs() <= MAX_SAFE_INTEGER
}

fn is_in_range(val: f64) -> bool {
    val.abs() <= MAX_SAFE_INTEGER
}

/// Number of digits after the decimal point in the shortest round-trip form.
pub(super) fn fraction_digits(val: f64) -> usize {
    let text = val.abs().to_string();
    text.find('.').map_or(0, |dot| text.len() - dot - 1)
}

impl ConstantFolder {
    /// The value of a numeric literal (optionally negated) within the safe-integer
    /// magnitude. `-0` is returned as `-0.0`.
    pub(super) fn extract_number(expr: &Expression<'_>) -> Option<f64> {
        match expr {
            Expression::NumericLiteral(lit) if is_in_range(lit.value) => Some(lit.value),
            Expression::UnaryExpression(unary) if unary.operator == UnaryOperator::UnaryNegation => {
                Self::extract_number(&unary.argument).map(|val| -val)
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

    /// Builds a literal for a folded result. `NaN`, infinities, values beyond the
    /// safe-integer magnitude and `-0` are not folded.
    pub(super) fn make_number<'a>(val: f64, ctx: &mut Ctx<'a>) -> Option<Expression<'a>> {
        if !is_in_range(val) || (val == 0.0 && val.is_sign_negative()) {
            return None;
        }
        let magnitude = val.abs();
        let raw = Some(oxc_allocator::FromIn::from_in(magnitude.to_string(), ctx.allocator()));
        let literal = Expression::new_numeric_literal(SPAN, magnitude, raw, NumberBase::Decimal, ctx);
        Some(if val < 0.0 {
            Expression::new_unary_expression(SPAN, UnaryOperator::UnaryNegation, literal, ctx)
        } else {
            literal
        })
    }

    pub(super) fn make_boolean<'a>(val: bool, ctx: &mut Ctx<'a>) -> Expression<'a> {
        Expression::new_boolean_literal(SPAN, val, ctx)
    }
}
