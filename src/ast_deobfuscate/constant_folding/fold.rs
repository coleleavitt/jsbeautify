use oxc_ast::ast::{
    BinaryExpression, BinaryOperator, Expression, LogicalExpression, LogicalOperator, UnaryExpression, UnaryOperator,
};

use super::values::fraction_digits;
use super::{ConstantFolder, Ctx};

/// ECMAScript `ToInt32` for an integer-valued, safe-range `f64`.
const fn to_int32(val: f64) -> i32 {
    val as i64 as i32
}

impl ConstantFolder {
    pub(super) fn try_fold_binary<'a>(
        &mut self,
        expr: &BinaryExpression<'a>,
        ctx: &mut Ctx<'a>,
    ) -> Option<Expression<'a>> {
        let left_val = Self::extract_number(&expr.left)?;
        let right_val = Self::extract_number(&expr.right)?;

        let result = match expr.operator {
            BinaryOperator::Addition => left_val + right_val,
            BinaryOperator::Subtraction => left_val - right_val,
            BinaryOperator::Multiplication => left_val * right_val,
            BinaryOperator::Division => left_val / right_val,
            BinaryOperator::Remainder => left_val % right_val,
            BinaryOperator::BitwiseAnd => f64::from(to_int32(left_val) & to_int32(right_val)),
            BinaryOperator::BitwiseOR => f64::from(to_int32(left_val) | to_int32(right_val)),
            BinaryOperator::BitwiseXOR => f64::from(to_int32(left_val) ^ to_int32(right_val)),
            BinaryOperator::ShiftLeft => f64::from(to_int32(left_val).wrapping_shl(to_int32(right_val) as u32)),
            BinaryOperator::ShiftRight => f64::from(to_int32(left_val).wrapping_shr(to_int32(right_val) as u32)),
            BinaryOperator::ShiftRightZeroFill => {
                f64::from((to_int32(left_val) as u32).wrapping_shr(to_int32(right_val) as u32))
            }
            _ => return None,
        };
        if fraction_digits(result) > fraction_digits(left_val) + fraction_digits(right_val) {
            return None;
        }
        let number = Self::make_number(result, ctx)?;

        eprintln!(
            "[AST] Folding: {} {:?} {} = {}",
            left_val, expr.operator, right_val, result
        );

        self.changed = true;
        Some(number)
    }

    pub(super) fn try_fold_comparison<'a>(
        &mut self,
        expr: &BinaryExpression<'a>,
        ctx: &mut Ctx<'a>,
    ) -> Option<Expression<'a>> {
        let left_val = Self::extract_number(&expr.left)?;
        let right_val = Self::extract_number(&expr.right)?;

        let result = match expr.operator {
            BinaryOperator::StrictEquality | BinaryOperator::Equality => left_val == right_val,
            BinaryOperator::StrictInequality | BinaryOperator::Inequality => left_val != right_val,
            BinaryOperator::LessThan => left_val < right_val,
            BinaryOperator::LessEqualThan => left_val <= right_val,
            BinaryOperator::GreaterThan => left_val > right_val,
            BinaryOperator::GreaterEqualThan => left_val >= right_val,
            _ => return None,
        };

        eprintln!(
            "[AST] Folding comparison: {} {:?} {} = {}",
            left_val, expr.operator, right_val, result
        );

        self.changed = true;
        Some(Self::make_boolean(result, ctx))
    }

    pub(super) fn try_fold_logical<'a>(
        &mut self,
        expr: &LogicalExpression<'a>,
        ctx: &mut Ctx<'a>,
    ) -> Option<Expression<'a>> {
        let left_bool = Self::extract_boolean(&expr.left)?;
        let right_bool = Self::extract_boolean(&expr.right)?;

        let result = match expr.operator {
            LogicalOperator::And => left_bool && right_bool,
            LogicalOperator::Or => left_bool || right_bool,
            LogicalOperator::Coalesce => return None,
        };

        eprintln!(
            "[AST] Folding logical: {} {:?} {} = {}",
            left_bool, expr.operator, right_bool, result
        );

        self.changed = true;
        Some(Self::make_boolean(result, ctx))
    }

    pub(super) fn try_fold_unary<'a>(
        &mut self,
        expr: &UnaryExpression<'a>,
        ctx: &mut Ctx<'a>,
    ) -> Option<Expression<'a>> {
        match expr.operator {
            UnaryOperator::UnaryNegation if !matches!(expr.argument, Expression::NumericLiteral(_)) => {
                let val = Self::extract_number(&expr.argument)?;
                let number = Self::make_number(-val, ctx)?;
                eprintln!("[AST] Folding unary: -{val}");
                self.changed = true;
                Some(number)
            }
            UnaryOperator::BitwiseNot => {
                let val = Self::extract_number(&expr.argument)?;
                let result = !to_int32(val);
                let number = Self::make_number(f64::from(result), ctx)?;
                eprintln!("[AST] Folding unary: ~{val} = {result}");
                self.changed = true;
                Some(number)
            }
            UnaryOperator::LogicalNot => {
                let val = Self::extract_boolean(&expr.argument)?;
                let result = !val;
                eprintln!("[AST] Folding unary: !{val} = {result}");
                self.changed = true;
                Some(Self::make_boolean(result, ctx))
            }
            UnaryOperator::UnaryPlus => {
                let val = Self::extract_number(&expr.argument)?;
                let number = Self::make_number(val, ctx)?;
                eprintln!("[AST] Folding unary: +{val}");
                self.changed = true;
                Some(number)
            }
            _ => None,
        }
    }
}
