use oxc_ast::ast::{BinaryOperator, Expression, UnaryOperator};
use oxc_span::SPAN;
use oxc_syntax::number::NumberBase;
use oxc_traverse::Traverse;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;

pub struct BooleanArithmeticFolder {
    folded: usize,
}

impl BooleanArithmeticFolder {
    #[must_use]
    pub const fn new() -> Self {
        Self { folded: 0 }
    }

    #[must_use]
    pub const fn folded_count(&self) -> usize {
        self.folded
    }

    fn extract_numlike(expr: &Expression<'_>) -> Option<f64> {
        match expr {
            Expression::NumericLiteral(n) => Some(n.value),
            Expression::BooleanLiteral(b) => Some(f64::from(u8::from(b.value))),
            Expression::UnaryExpression(u) if u.operator == UnaryOperator::UnaryPlus => {
                Self::extract_numlike(&u.argument)
            }
            Expression::UnaryExpression(u) if u.operator == UnaryOperator::UnaryNegation => {
                Self::extract_numlike(&u.argument).map(|v| -v)
            }
            _ => None,
        }
    }

    fn make_number<'a>(val: f64, ctx: &Ctx<'a>) -> Expression<'a> {
        Expression::new_numeric_literal(SPAN, val, None, NumberBase::Decimal, ctx)
    }
}

impl Default for BooleanArithmeticFolder {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for BooleanArithmeticFolder {
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        // 1. Unary: `+true`, `+false`, `-true`, `!true` are handled elsewhere but
        //    `+true` through `UnaryPlus` is not folded by constant_folder since
        //    it expects NumericLiteral only. Rewrite here.
        if let Expression::UnaryExpression(unary) = expr
            && unary.operator == UnaryOperator::UnaryPlus
            && let Expression::BooleanLiteral(b) = &unary.argument
        {
            let val = f64::from(u8::from(b.value));
            if self.folded < 20 {
                eprintln!("[AKAMAI/bool-arith] +{} -> {}", b.value, val as i64);
            }
            self.folded += 1;
            *expr = Self::make_number(val, ctx);
            return;
        }

        // 2. Binary between bool-likes: `true + true`, `true + 1`, etc.
        if let Expression::BinaryExpression(binary) = expr {
            let Some(l) = Self::extract_numlike(&binary.left) else {
                return;
            };
            let Some(r) = Self::extract_numlike(&binary.right) else {
                return;
            };
            // Only fold if at least ONE side was originally a Boolean (otherwise
            // constant_folder already handled it).
            let has_bool = contains_boolean_literal(&binary.left) || contains_boolean_literal(&binary.right);
            if !has_bool {
                return;
            }
            let result = match binary.operator {
                BinaryOperator::Addition => l + r,
                BinaryOperator::Subtraction => l - r,
                BinaryOperator::Multiplication => l * r,
                BinaryOperator::Division if r != 0.0 => l / r,
                _ => return,
            };
            if !result.is_finite() {
                return;
            }
            if self.folded < 20 {
                eprintln!("[AKAMAI/bool-arith] {l} {:?} {r} -> {}", binary.operator, result as i64);
            }
            self.folded += 1;
            *expr = Self::make_number(result, ctx);
        }
    }
}

fn contains_boolean_literal(expr: &Expression<'_>) -> bool {
    match expr {
        Expression::BooleanLiteral(_) => true,
        Expression::UnaryExpression(u) => contains_boolean_literal(&u.argument),
        _ => false,
    }
}
