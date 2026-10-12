use oxc_allocator::Vec as OxcVec;
use oxc_ast::ast::{
    BinaryOperator, Expression, IfStatement, LogicalExpression, LogicalOperator, Statement, UnaryOperator,
};
use oxc_semantic::ScopeFlags;
use oxc_span::SPAN;
use oxc_traverse::TraverseCtx;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub fn is_short_circuit_expr_stmt(stmt: &Statement<'_>) -> bool {
    matches!(
        stmt,
        Statement::ExpressionStatement(expr_stmt)
            if matches!(
                &expr_stmt.expression,
                Expression::LogicalExpression(logical)
                    if matches!(logical.operator, LogicalOperator::And | LogicalOperator::Or)
            )
    )
}

/// Strips the parentheses `Expression::without_parentheses` looks through,
/// keeping the inner expression.
pub fn into_without_parentheses(mut expr: Expression<'_>) -> Expression<'_> {
    while let Expression::ParenthesizedExpression(paren) = expr {
        expr = paren.unbox().expression;
    }
    expr
}

/// Rewrites `x && y;` to `if (x) { y; }` (and `x || y;` to `if (!x) { y; }`),
/// or gives `stmt` back when it is not such a statement.
pub fn short_circuit_to_if<'a>(stmt: Statement<'a>, ctx: &mut Ctx<'a>) -> Result<Statement<'a>, Statement<'a>> {
    if !is_short_circuit_expr_stmt(&stmt) {
        return Err(stmt);
    }
    let Statement::ExpressionStatement(expr_stmt) = stmt else {
        unreachable!("is_short_circuit_expr_stmt matched")
    };
    let Expression::LogicalExpression(logical) = expr_stmt.unbox().expression else {
        unreachable!("is_short_circuit_expr_stmt matched")
    };
    let LogicalExpression {
        left, operator, right, ..
    } = logical.unbox();
    let action_stmt = Statement::new_expression_statement(SPAN, into_without_parentheses(right), &ctx.ast);
    let mut body_stmts = OxcVec::new_in(ctx);
    body_stmts.push(action_stmt);
    let block_scope_id = ctx.create_child_scope_of_current(ScopeFlags::empty());
    let block = Statement::new_block_statement_with_scope_id(SPAN, body_stmts, block_scope_id, &ctx.ast);

    let test = match operator {
        LogicalOperator::And | LogicalOperator::Coalesce => left,
        LogicalOperator::Or => negate_condition(left, ctx),
    };

    Ok(Statement::IfStatement(IfStatement::boxed(
        SPAN, test, block, None, &ctx.ast,
    )))
}

/// Negates `expr` for a position where only its truthiness is observed (an
/// `if` test): `!x` becomes `x`, equality operators flip, and De Morgan is
/// applied when it cancels a negation on at least one operand.
fn negate_condition<'a>(expr: Expression<'a>, ctx: &mut Ctx<'a>) -> Expression<'a> {
    match into_without_parentheses(expr) {
        Expression::UnaryExpression(unary) if unary.operator == UnaryOperator::LogicalNot => unary.unbox().argument,
        Expression::BinaryExpression(binary) if negated_equality(binary.operator).is_some() => {
            let binary = binary.unbox();
            let operator = negated_equality(binary.operator).expect("guard checked the operator");
            Expression::new_binary_expression(SPAN, binary.left, operator, binary.right, &ctx.ast)
        }
        Expression::LogicalExpression(logical)
            if logical.operator != LogicalOperator::Coalesce
                && (cancels_negation(&logical.left) || cancels_negation(&logical.right)) =>
        {
            let logical = logical.unbox();
            let operator = if logical.operator == LogicalOperator::And {
                LogicalOperator::Or
            } else {
                LogicalOperator::And
            };
            let left = negate_condition(logical.left, ctx);
            let right = negate_condition(logical.right, ctx);
            Expression::new_logical_expression(SPAN, left, operator, right, &ctx.ast)
        }
        inner => Expression::new_unary_expression(SPAN, UnaryOperator::LogicalNot, inner, &ctx.ast),
    }
}

fn cancels_negation(expr: &Expression<'_>) -> bool {
    match expr.without_parentheses() {
        Expression::UnaryExpression(unary) => unary.operator == UnaryOperator::LogicalNot,
        Expression::BinaryExpression(binary) => negated_equality(binary.operator).is_some(),
        _ => false,
    }
}

/// `!(a OP b)` is exactly `a NOP b` for the equality operators; relational
/// operators have no such complement because of `NaN`.
pub const fn negated_equality(operator: BinaryOperator) -> Option<BinaryOperator> {
    match operator {
        BinaryOperator::Equality => Some(BinaryOperator::Inequality),
        BinaryOperator::Inequality => Some(BinaryOperator::Equality),
        BinaryOperator::StrictEquality => Some(BinaryOperator::StrictInequality),
        BinaryOperator::StrictInequality => Some(BinaryOperator::StrictEquality),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast_deobfuscate::statement_split::normalize;
    use oxc_span::SourceType;

    fn run_convert(code: &str) -> (String, usize) {
        normalize(code, SourceType::mjs())
    }

    #[test]
    fn test_convert_and() {
        let (output, count) = run_convert("cond && doA();");
        assert!(
            count >= 1,
            "Should have converted at least 1 && expression, got: {count}"
        );
        assert!(output.contains("if"), "Should contain 'if', got: {output}");
        assert!(!output.contains("&&"), "Should NOT contain '&&', got: {output}");
    }

    #[test]
    fn test_convert_or() {
        let (output, count) = run_convert("cond || doA();");
        assert!(
            count >= 1,
            "Should have converted at least 1 || expression, got: {count}"
        );
        assert!(output.contains("if"), "Should contain 'if', got: {output}");
        assert!(output.contains('!'), "Should contain '!' negation, got: {output}");
        assert!(!output.contains("||"), "Should NOT contain '||', got: {output}");
    }

    #[test]
    fn test_preserve_assignment_and() {
        let (output, count) = run_convert("var x = cond && val;");
        assert_eq!(count, 0, "Should NOT convert && inside var declaration, got: {count}");
        assert!(output.contains("&&"), "Should still contain '&&', got: {output}");
    }

    #[test]
    fn test_preserve_return_or() {
        let (output, count) = run_convert("function f() { return cond || val; }");
        assert_eq!(count, 0, "Should NOT convert || inside return, got: {count}");
        assert!(output.contains("||"), "Should still contain '||', got: {output}");
    }

    #[test]
    fn test_convert_in_function_body() {
        let (output, count) = run_convert("function f() { cond && doA(); }");
        assert!(count >= 1, "Should have converted && in function body, got: {count}");
        assert!(output.contains("if"), "Should contain 'if', got: {output}");
    }

    #[test]
    fn test_preserve_arrow_expression_body() {
        let (output, count) = run_convert("const f = () => cond && doA();");
        assert_eq!(count, 0, "Should NOT convert && in arrow expression body, got: {count}");
        assert!(output.contains("&&"), "Should still contain '&&', got: {output}");
    }

    #[test]
    fn test_preserve_nullish_coalescing() {
        let (output, count) = run_convert("x ?? (x = 1);");
        assert_eq!(count, 0, "Should NOT convert ?? (nullish coalescing), got: {count}");
        assert!(output.contains("??"), "Should still contain '??', got: {output}");
    }

    fn convert_first_if_test(code: &str) -> String {
        let (output, count) = run_convert(code);
        assert_eq!(count, 1, "expected one conversion: {output}");
        output.lines().next().unwrap_or_default().to_string()
    }

    #[test]
    fn or_of_negation_tests_the_operand() {
        assert_eq!(convert_first_if_test("!x || foo();"), "if (x) {");
    }

    #[test]
    fn or_of_equality_flips_the_operator() {
        assert_eq!(convert_first_if_test("a === b || foo();"), "if (a !== b) {");
        assert_eq!(convert_first_if_test("a != b || foo();"), "if (a == b) {");
    }

    #[test]
    fn or_of_relational_keeps_the_negation() {
        // `!(a < b)` is not `a >= b` when either side is NaN.
        assert_eq!(convert_first_if_test("a < b || foo();"), "if (!(a < b)) {");
    }

    #[test]
    fn de_morgan_only_when_it_cancels_a_negation() {
        assert_eq!(convert_first_if_test("!x || y || foo();"), "if (x && !y) {");
        assert_eq!(convert_first_if_test("x && y || foo();"), "if (!(x && y)) {");
    }

    #[test]
    fn action_loses_its_parentheses() {
        // The ternary must be a bare statement so it is converted as well.
        let (output, count) = run_convert("x && (y ? f() : g());");
        assert_eq!(count, 2, "got: {output}");
        assert_eq!(output, "if (x) {\n\tif (y) {\n\t\tf();\n\t} else {\n\t\tg();\n\t}\n}\n");
    }

    #[test]
    fn converts_in_switch_case() {
        let (output, count) = run_convert("switch (k) { case 1: a && b(); break; }");
        assert_eq!(count, 1, "got: {output}");
        assert!(!output.contains("&&"), "got: {output}");
    }

    #[test]
    fn converts_nested_short_circuits_in_one_traversal() {
        let (output, count) = run_convert("a && (b && c());");
        assert_eq!(count, 2, "got: {output}");
        assert!(!output.contains("&&"), "got: {output}");
    }
}
