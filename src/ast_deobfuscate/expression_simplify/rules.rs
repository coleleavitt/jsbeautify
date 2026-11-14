use oxc_allocator::GetAllocator;
use oxc_ast::ast::{
    BinaryExpression, BinaryOperator, ComputedMemberExpression, Expression, UnaryExpression, UnaryOperator,
};
use oxc_span::SPAN;
use oxc_traverse::Ancestor;

use super::{Ctx, ExpressionSimplifier};

impl ExpressionSimplifier {
    pub(super) fn try_simplify_not_number<'a>(
        &mut self,
        unary: &UnaryExpression<'a>,
        ctx: &mut Ctx<'a>,
    ) -> Option<Expression<'a>> {
        if unary.operator != UnaryOperator::LogicalNot {
            return None;
        }

        if let Expression::NumericLiteral(num) = &unary.argument {
            let result = num.value == 0.0;
            deob_trace!("[AST] Simplifying !{} to {}", num.value as i64, result);
            self.rewrites += 1;
            return Some(Self::make_boolean(result, ctx));
        }

        None
    }

    pub(super) fn try_simplify_double_not<'a>(
        &mut self,
        unary: &UnaryExpression<'a>,
        ctx: &mut Ctx<'a>,
    ) -> Option<Expression<'a>> {
        if unary.operator != UnaryOperator::LogicalNot {
            return None;
        }

        if let Expression::UnaryExpression(inner_unary) = &unary.argument {
            if inner_unary.operator != UnaryOperator::LogicalNot {
                return None;
            }

            if let Expression::ArrayExpression(arr) = &inner_unary.argument
                && arr.elements.is_empty()
            {
                deob_trace!("[AST] Simplifying !![] to true");
                self.rewrites += 1;
                return Some(Self::make_boolean(true, ctx));
            }

            if let Expression::NumericLiteral(num) = &inner_unary.argument {
                let result = num.value != 0.0;
                deob_trace!("[AST] Simplifying !!{} to {}", num.value as i64, result);
                self.rewrites += 1;
                return Some(Self::make_boolean(result, ctx));
            }
        }

        if let Expression::BooleanLiteral(bool_lit) = &unary.argument {
            let result = !bool_lit.value;
            deob_trace!("[AST] Simplifying !{} to {}", bool_lit.value, result);
            self.rewrites += 1;
            return Some(Self::make_boolean(result, ctx));
        }

        None
    }

    pub(super) fn try_simplify_plus_array<'a>(
        &mut self,
        unary: &UnaryExpression<'a>,
        ctx: &mut Ctx<'a>,
    ) -> Option<Expression<'a>> {
        if unary.operator != UnaryOperator::UnaryPlus {
            return None;
        }

        if let Expression::ArrayExpression(arr) = &unary.argument
            && arr.elements.is_empty()
        {
            deob_trace!("[AST] Simplifying +[] to 0");
            self.rewrites += 1;
            return Some(Self::make_number(0, ctx));
        }

        if let Expression::UnaryExpression(inner_unary) = &unary.argument
            && inner_unary.operator == UnaryOperator::LogicalNot
            && let Expression::ArrayExpression(arr) = &inner_unary.argument
            && arr.elements.is_empty()
        {
            deob_trace!("[AST] Simplifying +![] to 1");
            self.rewrites += 1;
            return Some(Self::make_number(1, ctx));
        }

        None
    }

    pub(super) fn try_simplify_bracket_to_dot<'a>(
        &mut self,
        member: &ComputedMemberExpression<'a>,
        ctx: &mut Ctx<'a>,
    ) -> Option<Expression<'a>> {
        if let Expression::StringLiteral(lit) = &member.expression {
            let prop_name = lit.value.as_str();

            if Self::is_valid_identifier(prop_name) {
                deob_trace!("[AST] Converting [\"{prop_name}\"]] to .{prop_name}");
                self.rewrites += 1;

                let object = Self::clone_expression(&member.object, ctx);
                let property = oxc_ast::ast::IdentifierName::new(SPAN, prop_name, ctx);
                return Some(Expression::new_static_member_expression(
                    SPAN,
                    object,
                    property,
                    member.optional,
                    ctx,
                ));
            }
        }

        None
    }

    /// Folds a `+` chain of string literals into one literal. `"a" + "b" + "c"`
    /// parses as `("a" + "b") + "c"`; the whole left spine is folded once at its
    /// top, since folding each `+` would allocate every prefix of the result.
    pub(super) fn try_simplify_string_concat<'a>(
        &mut self,
        binary: &BinaryExpression<'a>,
        ctx: &mut Ctx<'a>,
    ) -> Option<Expression<'a>> {
        if binary.operator != BinaryOperator::Addition {
            return None;
        }
        if let Ancestor::BinaryExpressionLeft(parent) = ctx.parent()
            && *parent.operator() == BinaryOperator::Addition
            && foldable_string(parent.right()).is_some()
        {
            return None;
        }

        let mut rights = Vec::new();
        let mut node = binary;
        let first = loop {
            rights.push(foldable_string(&node.right)?);
            match &node.left {
                Expression::BinaryExpression(inner) if inner.operator == BinaryOperator::Addition => node = inner,
                left => break foldable_string(left)?,
            }
        };
        let mut combined = String::with_capacity(first.len() + rights.iter().map(|s| s.len()).sum::<usize>());
        combined.push_str(first);
        for right in rights.iter().rev() {
            combined.push_str(right);
        }
        deob_trace!(
            "[AST] Concatenating {} string literals = \"{}\"",
            rights.len() + 1,
            combined
        );
        self.rewrites += rights.len();

        Some(Expression::new_string_literal(
            SPAN,
            ctx.allocator().alloc_str(&combined),
            None,
            ctx,
        ))
    }
}

fn foldable_string<'b>(expr: &'b Expression<'_>) -> Option<&'b str> {
    match expr {
        Expression::StringLiteral(lit) if !lit.lone_surrogates => Some(lit.value.as_str()),
        _ => None,
    }
}
