use oxc_ast::ast::{BinaryExpression, BinaryOperator, Expression};

use super::{AlgebraicSimplifier, Ctx};

impl AlgebraicSimplifier {
    pub(super) fn try_simplify_self_subtract<'a>(
        &mut self,
        binary: &BinaryExpression<'a>,
        ctx: &mut Ctx<'a>,
    ) -> Option<Expression<'a>> {
        if binary.operator != BinaryOperator::Subtraction {
            return None;
        }

        if let (Expression::Identifier(left), Expression::Identifier(right)) = (&binary.left, &binary.right)
            && left.name == right.name
        {
            eprintln!("[AST] Simplifying {} - {} to 0", left.name, right.name);
            self.changed = true;
            return Some(Self::make_number(0, ctx));
        }

        None
    }

    pub(super) fn try_simplify_multiply_zero<'a>(
        &mut self,
        binary: &BinaryExpression<'a>,
        ctx: &mut Ctx<'a>,
    ) -> Option<Expression<'a>> {
        if binary.operator != BinaryOperator::Multiplication {
            return None;
        }

        let left_is_zero = Self::is_zero(&binary.left);
        let right_is_zero = Self::is_zero(&binary.right);

        if left_is_zero || right_is_zero {
            eprintln!("[AST] Simplifying x * 0 to 0");
            self.changed = true;
            return Some(Self::make_number(0, ctx));
        }

        None
    }

    pub(super) fn try_simplify_self_divide<'a>(
        &mut self,
        binary: &BinaryExpression<'a>,
        ctx: &mut Ctx<'a>,
    ) -> Option<Expression<'a>> {
        if binary.operator != BinaryOperator::Division {
            return None;
        }

        if let (Expression::Identifier(left), Expression::Identifier(right)) = (&binary.left, &binary.right)
            && left.name == right.name
        {
            eprintln!("[AST] Simplifying {} / {} to 1", left.name, right.name);
            self.changed = true;
            return Some(Self::make_number(1, ctx));
        }

        None
    }

    pub(super) fn try_simplify_self_modulo<'a>(
        &mut self,
        binary: &BinaryExpression<'a>,
        ctx: &mut Ctx<'a>,
    ) -> Option<Expression<'a>> {
        if binary.operator != BinaryOperator::Remainder {
            return None;
        }

        if let (Expression::Identifier(left), Expression::Identifier(right)) = (&binary.left, &binary.right)
            && left.name == right.name
        {
            eprintln!("[AST] Simplifying {} % {} to 0", left.name, right.name);
            self.changed = true;
            return Some(Self::make_number(0, ctx));
        }

        None
    }

    pub(super) fn try_simplify_self_xor<'a>(
        &mut self,
        binary: &BinaryExpression<'a>,
        ctx: &mut Ctx<'a>,
    ) -> Option<Expression<'a>> {
        if binary.operator != BinaryOperator::BitwiseXOR {
            return None;
        }

        if let (Expression::Identifier(left), Expression::Identifier(right)) = (&binary.left, &binary.right)
            && left.name == right.name
        {
            eprintln!("[AST] Simplifying {} ^ {} to 0", left.name, right.name);
            self.changed = true;
            return Some(Self::make_number(0, ctx));
        }

        None
    }

    pub(super) fn try_simplify_add_zero<'a>(
        &mut self,
        binary: &BinaryExpression<'a>,
        ctx: &mut Ctx<'a>,
    ) -> Option<Expression<'a>> {
        if binary.operator != BinaryOperator::Addition {
            return None;
        }

        if Self::is_zero(&binary.left) {
            eprintln!("[AST] Simplifying 0 + x to x");
            self.changed = true;
            return Some(Self::clone_expression(&binary.right, ctx));
        }

        if Self::is_zero(&binary.right) {
            eprintln!("[AST] Simplifying x + 0 to x");
            self.changed = true;
            return Some(Self::clone_expression(&binary.left, ctx));
        }

        None
    }

    pub(super) fn try_simplify_multiply_one<'a>(
        &mut self,
        binary: &BinaryExpression<'a>,
        ctx: &mut Ctx<'a>,
    ) -> Option<Expression<'a>> {
        if binary.operator != BinaryOperator::Multiplication {
            return None;
        }

        if Self::is_one(&binary.left) {
            eprintln!("[AST] Simplifying 1 * x to x");
            self.changed = true;
            return Some(Self::clone_expression(&binary.right, ctx));
        }

        if Self::is_one(&binary.right) {
            eprintln!("[AST] Simplifying x * 1 to x");
            self.changed = true;
            return Some(Self::clone_expression(&binary.left, ctx));
        }

        None
    }

    pub(super) fn try_simplify_divide_one<'a>(
        &mut self,
        binary: &BinaryExpression<'a>,
        ctx: &mut Ctx<'a>,
    ) -> Option<Expression<'a>> {
        if binary.operator != BinaryOperator::Division {
            return None;
        }

        if Self::is_one(&binary.right) {
            eprintln!("[AST] Simplifying x / 1 to x");
            self.changed = true;
            return Some(Self::clone_expression(&binary.left, ctx));
        }

        None
    }
}
