//! Bun printer/minifier idiom normalizers.

use oxc_allocator::{CloneIn, GetAllocator};
use oxc_ast::ast::{BinaryExpression, BinaryOperator, Expression, UnaryOperator};
use oxc_span::SPAN;
use oxc_traverse::{Traverse, TraverseCtx};

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

#[derive(Default)]
pub struct BunSyntaxNormalizer {
    typeof_rewrites: usize,
    numeric_rewrites: usize,
}

impl BunSyntaxNormalizer {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub const fn typeof_rewrites(&self) -> usize {
        self.typeof_rewrites
    }

    #[must_use]
    pub const fn numeric_rewrites(&self) -> usize {
        self.numeric_rewrites
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for BunSyntaxNormalizer {
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        let Expression::BinaryExpression(binary) = expr else {
            return;
        };

        if let Some(replacement) = normalize_typeof_undefined(binary, ctx) {
            *expr = replacement;
            self.typeof_rewrites += 1;
            return;
        }

        if let Some(replacement) = normalize_numeric_idiom(binary, ctx) {
            *expr = replacement;
            self.numeric_rewrites += 1;
        }
    }
}

fn normalize_typeof_undefined<'a>(binary: &BinaryExpression<'a>, ctx: &Ctx<'a>) -> Option<Expression<'a>> {
    if is_typeof_expr(&binary.left) && is_u_string(&binary.right) {
        let op = match binary.operator {
            BinaryOperator::GreaterThan => BinaryOperator::StrictEquality,
            BinaryOperator::LessThan => BinaryOperator::StrictInequality,
            _ => return None,
        };
        return Some(Expression::new_binary_expression(
            SPAN,
            binary.left.clone_in_with_semantic_ids(ctx.allocator()),
            op,
            undefined_string(ctx),
            ctx,
        ));
    }

    if is_u_string(&binary.left) && is_typeof_expr(&binary.right) {
        let op = match binary.operator {
            BinaryOperator::LessThan => BinaryOperator::StrictEquality,
            BinaryOperator::GreaterThan => BinaryOperator::StrictInequality,
            _ => return None,
        };
        return Some(Expression::new_binary_expression(
            SPAN,
            binary.right.clone_in_with_semantic_ids(ctx.allocator()),
            op,
            undefined_string(ctx),
            ctx,
        ));
    }

    None
}

fn normalize_numeric_idiom<'a>(binary: &BinaryExpression<'a>, ctx: &Ctx<'a>) -> Option<Expression<'a>> {
    if binary.operator != BinaryOperator::Division || !is_positive_zero_literal(&binary.right) {
        return None;
    }

    match numeric_value(&binary.left)? {
        1.0 => Some(Expression::new_identifier(SPAN, "Infinity", ctx)),
        -1.0 => {
            let infinity = Expression::new_identifier(SPAN, "Infinity", ctx);
            Some(Expression::new_unary_expression(
                SPAN,
                UnaryOperator::UnaryNegation,
                infinity,
                ctx,
            ))
        }
        0.0 => Some(Expression::new_identifier(SPAN, "NaN", ctx)),
        _ => None,
    }
}

fn is_positive_zero_literal(expr: &Expression<'_>) -> bool {
    matches!(
        expr,
        Expression::NumericLiteral(lit)
            if lit.value == 0.0
                && !lit.value.is_sign_negative()
                && lit.raw.as_ref().is_none_or(|raw| !raw.as_str().starts_with('-'))
    )
}

fn undefined_string<'a>(ctx: &Ctx<'a>) -> Expression<'a> {
    Expression::new_string_literal(
        SPAN,
        "undefined",
        Some(oxc_ast::ast::Str::from_str_in("\"undefined\"", ctx)),
        ctx,
    )
}

fn is_typeof_expr(expr: &Expression<'_>) -> bool {
    matches!(expr, Expression::UnaryExpression(unary) if unary.operator == UnaryOperator::Typeof)
}

fn is_u_string(expr: &Expression<'_>) -> bool {
    matches!(expr, Expression::StringLiteral(lit) if lit.value.as_str() == "u")
}

fn numeric_value(expr: &Expression<'_>) -> Option<f64> {
    match expr {
        Expression::NumericLiteral(lit) => Some(lit.value),
        Expression::UnaryExpression(unary) if unary.operator == UnaryOperator::UnaryNegation => {
            Some(-numeric_value(&unary.argument)?)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast_deobfuscate::semantic;
    use oxc_allocator::Allocator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_span::SourceType;
    use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

    fn run(code: &str) -> String {
        let allocator = Allocator::default();
        let parsed = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let mut program = parsed.program;
        let scoping = semantic::scoping(&program);
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        let mut pass = BunSyntaxNormalizer::new();
        traverse_mut_with_ctx(&mut pass, &mut program, &mut ctx);
        Codegen::new().build(&program).code
    }

    #[test]
    fn normalizes_typeof_less_than_u() {
        let out = run("if (typeof process < \"u\") ok();");
        assert!(out.contains("typeof process !== \"undefined\""), "got: {out}");
    }

    #[test]
    fn normalizes_typeof_greater_than_u() {
        let out = run("if (typeof process > \"u\") ok();");
        assert!(out.contains("typeof process === \"undefined\""), "got: {out}");
    }

    #[test]
    fn normalizes_numeric_idioms() {
        let out = run("var a = 1 / 0; var b = -1 / 0; var c = 0 / 0;");
        assert!(out.contains("Infinity"), "got: {out}");
        assert!(out.contains("-Infinity"), "got: {out}");
        assert!(out.contains("NaN"), "got: {out}");
    }

    #[test]
    fn leaves_signed_zero_division_untouched() {
        let out = run("var a = 1 / -0; var b = -1 / -0;");
        assert!(out.contains("1 / -0") || out.contains("1/-0"), "got: {out}");
        assert!(out.contains("-1 / -0") || out.contains("-1/-0"), "got: {out}");
    }
}
