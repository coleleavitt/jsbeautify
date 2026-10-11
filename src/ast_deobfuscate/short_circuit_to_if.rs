use oxc_allocator::{CloneIn, GetAllocator, Vec as OxcVec};
use oxc_ast::ast::{
    BinaryOperator, BlockStatement, Expression, FunctionBody, IfStatement, LogicalExpression, LogicalOperator, Program,
    Statement, SwitchCase, UnaryOperator,
};
use oxc_semantic::ScopeFlags;
use oxc_span::SPAN;
use oxc_traverse::{Traverse, TraverseCtx};

use crate::ast_deobfuscate::pass::counter_pass;
use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct ShortCircuitToIf {
    converted_count: usize,
}

counter_pass!(ShortCircuitToIf, converted_count, converted_count);

pub(crate) fn is_short_circuit_expr_stmt(stmt: &Statement<'_>) -> bool {
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

fn build_if_from_logical<'a>(logical: &LogicalExpression<'a>, ctx: &mut Ctx<'a>) -> Statement<'a> {
    let action_expr = logical
        .right
        .without_parentheses()
        .clone_in_with_semantic_ids(ctx.allocator());
    let action_stmt = Statement::new_expression_statement(SPAN, action_expr, &ctx.ast);
    let mut body_stmts = OxcVec::new_in(ctx);
    body_stmts.push(action_stmt);
    let block_scope_id = ctx.create_child_scope_of_current(ScopeFlags::empty());
    let block = Statement::new_block_statement_with_scope_id(SPAN, body_stmts, block_scope_id, &ctx.ast);

    let test = match logical.operator {
        LogicalOperator::And | LogicalOperator::Coalesce => logical.left.clone_in_with_semantic_ids(ctx.allocator()),
        LogicalOperator::Or => negate_condition(&logical.left, ctx),
    };

    Statement::IfStatement(IfStatement::boxed(SPAN, test, block, None, &ctx.ast))
}

/// Negates `expr` for a position where only its truthiness is observed (an
/// `if` test): `!x` becomes `x`, equality operators flip, and De Morgan is
/// applied when it cancels a negation on at least one operand.
fn negate_condition<'a>(expr: &Expression<'a>, ctx: &mut Ctx<'a>) -> Expression<'a> {
    let inner = expr.without_parentheses();
    match inner {
        Expression::UnaryExpression(unary) if unary.operator == UnaryOperator::LogicalNot => {
            return unary.argument.clone_in_with_semantic_ids(ctx.allocator());
        }
        Expression::BinaryExpression(binary) => {
            if let Some(operator) = negated_equality(binary.operator) {
                let left = binary.left.clone_in_with_semantic_ids(ctx.allocator());
                let right = binary.right.clone_in_with_semantic_ids(ctx.allocator());
                return Expression::new_binary_expression(SPAN, left, operator, right, &ctx.ast);
            }
        }
        Expression::LogicalExpression(logical)
            if logical.operator != LogicalOperator::Coalesce
                && (cancels_negation(&logical.left) || cancels_negation(&logical.right)) =>
        {
            let operator = if logical.operator == LogicalOperator::And {
                LogicalOperator::Or
            } else {
                LogicalOperator::And
            };
            let left = negate_condition(&logical.left, ctx);
            let right = negate_condition(&logical.right, ctx);
            return Expression::new_logical_expression(SPAN, left, operator, right, &ctx.ast);
        }
        _ => {}
    }
    let inner = inner.clone_in_with_semantic_ids(ctx.allocator());
    Expression::new_unary_expression(SPAN, UnaryOperator::LogicalNot, inner, &ctx.ast)
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
pub(crate) const fn negated_equality(operator: BinaryOperator) -> Option<BinaryOperator> {
    match operator {
        BinaryOperator::Equality => Some(BinaryOperator::Inequality),
        BinaryOperator::Inequality => Some(BinaryOperator::Equality),
        BinaryOperator::StrictEquality => Some(BinaryOperator::StrictInequality),
        BinaryOperator::StrictInequality => Some(BinaryOperator::StrictEquality),
        _ => None,
    }
}

impl ShortCircuitToIf {
    fn convert_statements<'a>(&mut self, stmts: &mut OxcVec<'a, Statement<'a>>, ctx: &mut Ctx<'a>) {
        if !stmts.iter().any(is_short_circuit_expr_stmt) {
            return;
        }

        let mut new_stmts = OxcVec::new_in(ctx);
        for stmt in stmts.iter() {
            if let Statement::ExpressionStatement(expr_stmt) = stmt
                && let Expression::LogicalExpression(logical) = &expr_stmt.expression
                && matches!(logical.operator, LogicalOperator::And | LogicalOperator::Or)
            {
                new_stmts.push(build_if_from_logical(logical, ctx));
                self.converted_count += 1;
                continue;
            }
            new_stmts.push(stmt.clone_in_with_semantic_ids(ctx.allocator()));
        }
        *stmts = new_stmts;
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for ShortCircuitToIf {
    fn exit_program(&mut self, program: &mut Program<'a>, ctx: &mut Ctx<'a>) {
        self.convert_statements(&mut program.body, ctx);
    }

    fn exit_block_statement(&mut self, block: &mut BlockStatement<'a>, ctx: &mut Ctx<'a>) {
        self.convert_statements(&mut block.body, ctx);
    }

    fn exit_function_body(&mut self, body: &mut FunctionBody<'a>, ctx: &mut Ctx<'a>) {
        self.convert_statements(&mut body.statements, ctx);
    }

    fn exit_switch_case(&mut self, case: &mut SwitchCase<'a>, ctx: &mut Ctx<'a>) {
        self.convert_statements(&mut case.consequent, ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast_deobfuscate::DeobfuscateState;
    use oxc_allocator::Allocator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_semantic::SemanticBuilder;
    use oxc_span::SourceType;
    use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

    fn run_convert(code: &str) -> (String, usize) {
        let allocator = Allocator::default();
        let source_type = SourceType::mjs();
        let ret = Parser::new(&allocator, code, source_type).parse();
        let mut program = ret.program;

        let mut converter = ShortCircuitToIf::new();
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

        traverse_mut_with_ctx(&mut converter, &mut program, &mut ctx);

        (Codegen::new().build(&program).code, converter.converted_count())
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
        let allocator = Allocator::default();
        let mut program = Parser::new(&allocator, "x && (y ? f() : g());", SourceType::mjs())
            .parse()
            .program;
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        traverse_mut_with_ctx(&mut ShortCircuitToIf::new(), &mut program, &mut ctx);
        let Statement::IfStatement(if_stmt) = &program.body[0] else {
            panic!("expected an if statement");
        };
        let Statement::BlockStatement(block) = &if_stmt.consequent else {
            panic!("expected a block body");
        };
        assert!(
            matches!(
                &block.body[0],
                Statement::ExpressionStatement(stmt) if matches!(stmt.expression, Expression::ConditionalExpression(_))
            ),
            "the ternary must be a bare statement so ternary_to_if_else can convert it"
        );
    }

    #[test]
    fn converts_in_switch_case() {
        let (output, count) = run_convert("switch (k) { case 1: a && b(); break; }");
        assert_eq!(count, 1, "got: {output}");
        assert!(!output.contains("&&"), "got: {output}");
    }
}
