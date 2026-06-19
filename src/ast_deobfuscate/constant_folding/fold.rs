use oxc_ast::ast::{
    BinaryExpression, BinaryOperator, Expression, LogicalExpression, LogicalOperator, UnaryExpression, UnaryOperator,
};

use super::{ConstantFolder, Ctx};

impl ConstantFolder {
    pub(super) fn try_fold_binary<'a>(
        &mut self,
        expr: &BinaryExpression<'a>,
        ctx: &mut Ctx<'a>,
    ) -> Option<Expression<'a>> {
        let left_val = Self::extract_number(&expr.left)?;
        let right_val = Self::extract_number(&expr.right)?;

        let result = match expr.operator {
            BinaryOperator::Addition => left_val.checked_add(right_val)?,
            BinaryOperator::Subtraction => left_val.checked_sub(right_val)?,
            BinaryOperator::Multiplication => left_val.checked_mul(right_val)?,
            BinaryOperator::Division => {
                if right_val == 0 {
                    return None;
                }
                left_val.checked_div(right_val)?
            }
            BinaryOperator::Remainder => {
                if right_val == 0 {
                    return None;
                }
                left_val.checked_rem(right_val)?
            }
            BinaryOperator::BitwiseAnd => left_val & right_val,
            BinaryOperator::BitwiseOR => left_val | right_val,
            BinaryOperator::BitwiseXOR => left_val ^ right_val,
            BinaryOperator::ShiftLeft => {
                let shift = (right_val & 0x1F) as u32;
                (left_val as i32).checked_shl(shift)? as i64
            }
            BinaryOperator::ShiftRight => {
                let shift = (right_val & 0x1F) as u32;
                (left_val as i32).checked_shr(shift)? as i64
            }
            BinaryOperator::ShiftRightZeroFill => {
                let shift = (right_val & 0x1F) as u32;
                ((left_val as u32) >> shift) as i64
            }
            _ => return None,
        };

        eprintln!(
            "[AST] Folding: {} {:?} {} = {}",
            left_val, expr.operator, right_val, result
        );

        self.changed = true;
        Some(Self::make_number(result, ctx))
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
            UnaryOperator::UnaryNegation => {
                let val = Self::extract_number(&expr.argument)?;
                let result = val.checked_neg()?;
                eprintln!("[AST] Folding unary: -{val} = {result}");
                self.changed = true;
                Some(Self::make_number(result, ctx))
            }
            UnaryOperator::BitwiseNot => {
                let val = Self::extract_number(&expr.argument)?;
                let result = !val;
                eprintln!("[AST] Folding unary: ~{val} = {result}");
                self.changed = true;
                Some(Self::make_number(result, ctx))
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
                eprintln!("[AST] Folding unary: +{val} = {val}");
                self.changed = true;
                Some(Self::make_number(val, ctx))
            }
            _ => None,
        }
    }
}
