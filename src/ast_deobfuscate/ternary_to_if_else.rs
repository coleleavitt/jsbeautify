//! Ternary-to-if/else conversion pass
//!
//! Converts standalone ternary expression statements into if/else statements
//! for improved readability. For example:
//!
//! ```js
//! cond ? doA() : doB();
//! ```
//!
//! Becomes:
//!
//! ```js
//! if (cond) {
//!   doA();
//! } else {
//!   doB();
//! }
//! ```
//!
//! Only converts when the `ConditionalExpression` is the direct expression of
//! an `ExpressionStatement`. Ternaries inside return statements, variable
//! declarations, assignments, or any other context are left untouched.

use oxc_allocator::{ArenaVec, CloneIn, GetAllocator};
use oxc_ast::ast::{BlockStatement, ConditionalExpression, Expression, FunctionBody, Program, Statement};
use oxc_semantic::ScopeFlags;
use oxc_span::SPAN;
use oxc_traverse::{Ancestor, Traverse, TraverseCtx};

use crate::ast_deobfuscate::pass::counter_pass;
use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct TernaryToIfElse {
    converted_count: usize,
}

counter_pass!(TernaryToIfElse, converted_count, converted_count);

/// Check whether a statement is an `ExpressionStatement` whose expression
/// is a `ConditionalExpression` (ternary).
fn is_ternary_expr_stmt(stmt: &Statement<'_>) -> bool {
    matches!(
        stmt,
        Statement::ExpressionStatement(expr_stmt)
            if matches!(expr_stmt.expression, Expression::ConditionalExpression(_))
    )
}

/// Build an `IfStatement` from a `ConditionalExpression`, wrapping the
/// consequent and alternate branches in `BlockStatement`s.
fn build_if_stmt<'a>(cond: &ConditionalExpression<'a>, ctx: &mut Ctx<'a>) -> Statement<'a> {
    let test = cond.test.clone_in_with_semantic_ids(ctx.ast.allocator());
    let consequent_expr = cond.consequent.clone_in_with_semantic_ids(ctx.ast.allocator());
    let alternate_expr = cond.alternate.clone_in_with_semantic_ids(ctx.ast.allocator());

    let consequent_stmt = Statement::new_expression_statement(SPAN, consequent_expr, &ctx.ast);
    let mut consequent_body = ArenaVec::new_in(&ctx.ast);
    consequent_body.push(consequent_stmt);
    let consequent_scope_id = ctx.create_child_scope_of_current(ScopeFlags::empty());
    let consequent_block =
        Statement::new_block_statement_with_scope_id(SPAN, consequent_body, consequent_scope_id, &ctx.ast);

    let alternate_stmt = Statement::new_expression_statement(SPAN, alternate_expr, &ctx.ast);
    let mut alternate_body = ArenaVec::new_in(&ctx.ast);
    alternate_body.push(alternate_stmt);
    let alternate_scope_id = ctx.create_child_scope_of_current(ScopeFlags::empty());
    let alternate_block =
        Statement::new_block_statement_with_scope_id(SPAN, alternate_body, alternate_scope_id, &ctx.ast);

    Statement::new_if_statement(SPAN, test, consequent_block, Some(alternate_block), &ctx.ast)
}

impl<'a> Traverse<'a, DeobfuscateState> for TernaryToIfElse {
    fn exit_program(&mut self, program: &mut Program<'a>, ctx: &mut Ctx<'a>) {
        let has_candidate = program.body.iter().any(|s| is_ternary_expr_stmt(s));
        if !has_candidate {
            return;
        }

        let before = program.body.len();
        let mut new_body = ArenaVec::new_in(&ctx.ast);
        for stmt in &program.body {
            if let Statement::ExpressionStatement(expr_stmt) = stmt
                && let Expression::ConditionalExpression(cond) = &expr_stmt.expression
            {
                eprintln!("[TERNARY_TO_IF] Converting ternary to if/else in program body");
                new_body.push(build_if_stmt(cond, ctx));
                self.converted_count += 1;
                continue;
            }
            new_body.push(stmt.clone_in_with_semantic_ids(ctx.ast.allocator()));
        }
        let after = new_body.len();
        eprintln!("[TERNARY_TO_IF] Program body: {before} -> {after} statements");
        program.body = new_body;
    }

    fn exit_block_statement(&mut self, block: &mut BlockStatement<'a>, ctx: &mut Ctx<'a>) {
        let has_candidate = block.body.iter().any(|s| is_ternary_expr_stmt(s));
        if !has_candidate {
            return;
        }

        let before = block.body.len();
        let mut new_body = ArenaVec::new_in(&ctx.ast);
        for stmt in &block.body {
            if let Statement::ExpressionStatement(expr_stmt) = stmt
                && let Expression::ConditionalExpression(cond) = &expr_stmt.expression
            {
                eprintln!("[TERNARY_TO_IF] Converting ternary to if/else in block");
                new_body.push(build_if_stmt(cond, ctx));
                self.converted_count += 1;
                continue;
            }
            new_body.push(stmt.clone_in_with_semantic_ids(ctx.ast.allocator()));
        }
        let after = new_body.len();
        eprintln!("[TERNARY_TO_IF] Block body: {before} -> {after} statements");
        block.body = new_body;
    }

    fn exit_function_body(&mut self, body: &mut FunctionBody<'a>, ctx: &mut Ctx<'a>) {
        // Skip arrow expression bodies — converting the ternary there would lose the
        // implicit return value. e.g. `(K) => cond ? a : b` must NOT become
        // `(K) => { if (cond) { a; } else { b; } }` because that changes semantics.
        if let Ancestor::ArrowFunctionExpressionBody(_) = ctx.parent() {
            // FunctionBody is visited only for block-bodied arrows in Oxc 0.148.
            return;
        }

        let has_candidate = body.statements.iter().any(|s| is_ternary_expr_stmt(s));
        if !has_candidate {
            return;
        }

        let before = body.statements.len();
        let mut new_stmts = ArenaVec::new_in(&ctx.ast);
        for stmt in &body.statements {
            if let Statement::ExpressionStatement(expr_stmt) = stmt
                && let Expression::ConditionalExpression(cond) = &expr_stmt.expression
            {
                eprintln!("[TERNARY_TO_IF] Converting ternary to if/else in function body");
                new_stmts.push(build_if_stmt(cond, ctx));
                self.converted_count += 1;
                continue;
            }
            new_stmts.push(stmt.clone_in_with_semantic_ids(ctx.ast.allocator()));
        }
        let after = new_stmts.len();
        eprintln!("[TERNARY_TO_IF] Function body: {before} -> {after} statements");
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

        let mut converter = TernaryToIfElse::new();
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

        traverse_mut_with_ctx(&mut converter, &mut program, &mut ctx);

        (Codegen::new().build(&program).code, converter.converted_count())
    }

    #[test]
    fn test_convert_simple_ternary() {
        let (output, count) = run_convert("cond ? doA() : doB();");
        assert!(count >= 1, "Should have converted at least 1 ternary, got: {count}");
        assert!(output.contains("if"), "Should contain 'if', got: {output}");
        assert!(output.contains("else"), "Should contain 'else', got: {output}");
        assert!(
            !output.contains('?'),
            "Should NOT contain '?' ternary operator, got: {output}"
        );
    }

    #[test]
    fn test_preserve_assignment_ternary() {
        let (output, count) = run_convert("var x = cond ? 1 : 2;");
        assert_eq!(
            count, 0,
            "Should NOT convert ternary inside var declaration, got: {count}"
        );
        assert!(
            output.contains('?'),
            "Should still contain '?' ternary operator, got: {output}"
        );
    }

    #[test]
    fn test_preserve_return_ternary() {
        let (output, count) = run_convert("function f() { return cond ? 1 : 2; }");
        assert_eq!(count, 0, "Should NOT convert ternary inside return, got: {count}");
        assert!(
            output.contains('?'),
            "Should still contain '?' ternary operator, got: {output}"
        );
    }

    #[test]
    fn test_convert_in_function_body() {
        let (output, count) = run_convert("function f() { cond ? doA() : doB(); }");
        assert!(
            count >= 1,
            "Should have converted ternary in function body, got: {count}"
        );
        assert!(output.contains("if"), "Should contain 'if', got: {output}");
    }

    #[test]
    fn test_convert_in_block() {
        let (output, count) = run_convert("{ cond ? doA() : doB(); }");
        assert!(count >= 1, "Should have converted ternary in block, got: {count}");
        assert!(output.contains("if"), "Should contain 'if', got: {output}");
    }

    #[test]
    fn test_preserve_arrow_expression_body() {
        let (output, count) = run_convert("const f = (K) => K ? 1 : 2;");
        assert_eq!(
            count, 0,
            "Should NOT convert ternary in arrow expression body, got: {count}"
        );
        assert!(
            output.contains('?'),
            "Should still contain '?' ternary operator, got: {output}"
        );
    }
}
