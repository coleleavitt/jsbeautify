use oxc_allocator::GetAllocator;
use oxc_ast::ast::{
    BinaryExpression, BinaryOperator, ComputedMemberExpression, Expression, UnaryExpression, UnaryOperator,
};
use oxc_span::SPAN;

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
            eprintln!("[AST] Simplifying !{} to {}", num.value as i64, result);
            self.changed = true;
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
                eprintln!("[AST] Simplifying !![] to true");
                self.changed = true;
                return Some(Self::make_boolean(true, ctx));
            }

            if let Expression::NumericLiteral(num) = &inner_unary.argument {
                let result = num.value != 0.0;
                eprintln!("[AST] Simplifying !!{} to {}", num.value as i64, result);
                self.changed = true;
                return Some(Self::make_boolean(result, ctx));
            }
        }

        if let Expression::BooleanLiteral(bool_lit) = &unary.argument {
            let result = !bool_lit.value;
            eprintln!("[AST] Simplifying !{} to {}", bool_lit.value, result);
            self.changed = true;
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
            eprintln!("[AST] Simplifying +[] to 0");
            self.changed = true;
            return Some(Self::make_number(0, ctx));
        }

        if let Expression::UnaryExpression(inner_unary) = &unary.argument
            && inner_unary.operator == UnaryOperator::LogicalNot
            && let Expression::ArrayExpression(arr) = &inner_unary.argument
            && arr.elements.is_empty()
        {
            eprintln!("[AST] Simplifying +![] to 1");
            self.changed = true;
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
                eprintln!("[AST] Converting [\"{prop_name}\"]] to .{prop_name}");
                self.changed = true;

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

    pub(super) fn try_simplify_string_concat<'a>(
        &mut self,
        binary: &BinaryExpression<'a>,
        ctx: &mut Ctx<'a>,
    ) -> Option<Expression<'a>> {
        if binary.operator != BinaryOperator::Addition {
            return None;
        }

        if let (Expression::StringLiteral(left), Expression::StringLiteral(right)) = (&binary.left, &binary.right)
            && !left.lone_surrogates
            && !right.lone_surrogates
        {
            let combined = format!("{}{}", left.value.as_str(), right.value.as_str());
            eprintln!(
                "[AST] Concatenating \"{}\" + \"{}\" = \"{}\"",
                left.value.as_str(),
                right.value.as_str(),
                combined
            );
            self.changed = true;

            return Some(Expression::new_string_literal(
                SPAN,
                ctx.allocator().alloc_str(&combined),
                None,
                ctx,
            ));
        }

        None
    }
}
