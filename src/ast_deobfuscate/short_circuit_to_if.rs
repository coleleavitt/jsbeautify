use oxc_allocator::CloneIn;
use oxc_ast::ast::{
    BlockStatement, Expression, ExpressionStatement, FunctionBody, IfStatement, LogicalExpression, LogicalOperator,
    Program, Statement, UnaryExpression, UnaryOperator,
};
use oxc_semantic::ScopeFlags;
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use oxc_traverse::{Ancestor, Traverse, TraverseCtx};
use std::cell::Cell;

use crate::ast_deobfuscate::pass::counter_pass;
use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct ShortCircuitToIf {
    converted_count: usize,
}

counter_pass!(ShortCircuitToIf, converted_count, converted_count);

fn is_short_circuit_expr_stmt(stmt: &Statement<'_>) -> bool {
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
    let action_expr = logical.right.clone_in_with_semantic_ids(ctx.ast.allocator);
    let action_stmt = Statement::ExpressionStatement(ctx.ast.alloc(ExpressionStatement {
        node_id: Cell::new(NodeId::DUMMY),
        span: SPAN,
        expression: action_expr,
    }));
    let mut body_stmts = ctx.ast.vec();
    body_stmts.push(action_stmt);
    let block_scope_id = ctx.create_child_scope_of_current(ScopeFlags::empty());
    let block = ctx.ast.statement_block_with_scope_id(SPAN, body_stmts, block_scope_id);

    let test = match logical.operator {
        LogicalOperator::And | LogicalOperator::Coalesce => logical.left.clone_in_with_semantic_ids(ctx.ast.allocator),
        LogicalOperator::Or => {
            let inner = logical.left.clone_in_with_semantic_ids(ctx.ast.allocator);
            Expression::UnaryExpression(ctx.ast.alloc(UnaryExpression {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                operator: UnaryOperator::LogicalNot,
                argument: inner,
            }))
        }
    };

    Statement::IfStatement(ctx.ast.alloc(IfStatement {
        node_id: Cell::new(NodeId::DUMMY),
        span: SPAN,
        test,
        consequent: block,
        alternate: None,
    }))
}

impl<'a> Traverse<'a, DeobfuscateState> for ShortCircuitToIf {
    fn exit_program(&mut self, program: &mut Program<'a>, ctx: &mut Ctx<'a>) {
        let has_candidate = program.body.iter().any(|s| is_short_circuit_expr_stmt(s));
        if !has_candidate {
            return;
        }

        let before = program.body.len();
        let mut new_body = ctx.ast.vec();
        for stmt in &program.body {
            if let Statement::ExpressionStatement(expr_stmt) = stmt
                && let Expression::LogicalExpression(logical) = &expr_stmt.expression
                && matches!(logical.operator, LogicalOperator::And | LogicalOperator::Or)
            {
                eprintln!(
                    "[SHORT_CIRCUIT_TO_IF] Converting {:?} to if in program body",
                    logical.operator
                );
                new_body.push(build_if_from_logical(logical, ctx));
                self.converted_count += 1;
                continue;
            }
            new_body.push(stmt.clone_in_with_semantic_ids(ctx.ast.allocator));
        }
        let after = new_body.len();
        eprintln!("[SHORT_CIRCUIT_TO_IF] Program body: {before} -> {after} statements");
        program.body = new_body;
    }

    fn exit_block_statement(&mut self, block: &mut BlockStatement<'a>, ctx: &mut Ctx<'a>) {
        let has_candidate = block.body.iter().any(|s| is_short_circuit_expr_stmt(s));
        if !has_candidate {
            return;
        }

        let before = block.body.len();
        let mut new_body = ctx.ast.vec();
        for stmt in &block.body {
            if let Statement::ExpressionStatement(expr_stmt) = stmt
                && let Expression::LogicalExpression(logical) = &expr_stmt.expression
                && matches!(logical.operator, LogicalOperator::And | LogicalOperator::Or)
            {
                eprintln!("[SHORT_CIRCUIT_TO_IF] Converting {:?} to if in block", logical.operator);
                new_body.push(build_if_from_logical(logical, ctx));
                self.converted_count += 1;
                continue;
            }
            new_body.push(stmt.clone_in_with_semantic_ids(ctx.ast.allocator));
        }
        let after = new_body.len();
        eprintln!("[SHORT_CIRCUIT_TO_IF] Block body: {before} -> {after} statements");
        block.body = new_body;
    }

    fn exit_function_body(&mut self, body: &mut FunctionBody<'a>, ctx: &mut Ctx<'a>) {
        // Skip arrow expression bodies — converting there would lose the
        // implicit return value and change semantics.
        if let Ancestor::ArrowFunctionExpressionBody(arrow) = ctx.parent()
            && *arrow.expression()
        {
            return;
        }

        let has_candidate = body.statements.iter().any(|s| is_short_circuit_expr_stmt(s));
        if !has_candidate {
            return;
        }

        let before = body.statements.len();
        let mut new_stmts = ctx.ast.vec();
        for stmt in &body.statements {
            if let Statement::ExpressionStatement(expr_stmt) = stmt
                && let Expression::LogicalExpression(logical) = &expr_stmt.expression
                && matches!(logical.operator, LogicalOperator::And | LogicalOperator::Or)
            {
                eprintln!(
                    "[SHORT_CIRCUIT_TO_IF] Converting {:?} to if in function body",
                    logical.operator
                );
                new_stmts.push(build_if_from_logical(logical, ctx));
                self.converted_count += 1;
                continue;
            }
            new_stmts.push(stmt.clone_in_with_semantic_ids(ctx.ast.allocator));
        }
        let after = new_stmts.len();
        eprintln!("[SHORT_CIRCUIT_TO_IF] Function body: {before} -> {after} statements");
        body.statements = new_stmts;
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
}
