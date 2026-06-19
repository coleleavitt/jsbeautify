use oxc_ast::ast::{Expression, LogicalExpression, LogicalOperator};

use super::{AlgebraicSimplifier, Ctx};

impl AlgebraicSimplifier {
    pub(super) fn try_simplify_logical_and<'a>(
        &mut self,
        logical: &LogicalExpression<'a>,
        ctx: &mut Ctx<'a>,
    ) -> Option<Expression<'a>> {
        if logical.operator != LogicalOperator::And {
            return None;
        }

        if Self::is_true(&logical.left) {
            eprintln!("[AST] Simplifying true && x to x");
            self.changed = true;
            return Some(Self::clone_expression(&logical.right, ctx));
        }

        if Self::is_false(&logical.left) {
            eprintln!("[AST] Simplifying false && x to false");
            self.changed = true;
            return Some(Self::make_boolean(false, ctx));
        }

        if Self::is_true(&logical.right) {
            eprintln!("[AST] Simplifying x && true to x");
            self.changed = true;
            return Some(Self::clone_expression(&logical.left, ctx));
        }

        if Self::is_false(&logical.right) {
            eprintln!("[AST] Simplifying x && false to false");
            self.changed = true;
            return Some(Self::make_boolean(false, ctx));
        }

        None
    }

    pub(super) fn try_simplify_logical_or<'a>(
        &mut self,
        logical: &LogicalExpression<'a>,
        ctx: &mut Ctx<'a>,
    ) -> Option<Expression<'a>> {
        if logical.operator != LogicalOperator::Or {
            return None;
        }

        if Self::is_true(&logical.left) {
            eprintln!("[AST] Simplifying true || x to true");
            self.changed = true;
            return Some(Self::make_boolean(true, ctx));
        }

        if Self::is_false(&logical.left) {
            eprintln!("[AST] Simplifying false || x to x");
            self.changed = true;
            return Some(Self::clone_expression(&logical.right, ctx));
        }

        if Self::is_true(&logical.right) {
            eprintln!("[AST] Simplifying x || true to true");
            self.changed = true;
            return Some(Self::make_boolean(true, ctx));
        }

        if Self::is_false(&logical.right) {
            eprintln!("[AST] Simplifying x || false to x");
            self.changed = true;
            return Some(Self::clone_expression(&logical.left, ctx));
        }

        None
    }
}
