//! Sequence expression splitting pass
//!
//! Splits `SequenceExpression` nodes used as `ExpressionStatement` into
//! individual `ExpressionStatement` nodes. For example:
//!
//! ```js
//! a = 1, b = 2, c = 3;  // single ExpressionStatement with SequenceExpression
//! ```
//!
//! Becomes:
//!
//! ```js
//! a = 1;
//! b = 2;
//! c = 3;  // three separate ExpressionStatements
//! ```
//!
//! Only splits when the `SequenceExpression` is the DIRECT child of an
//! `ExpressionStatement`. Sequences inside `for` init, `return`, ternary
//! conditions, etc. are left untouched.

use oxc_allocator::{CloneIn, GetAllocator, Vec as OxcVec};
use oxc_ast::ast::{BlockStatement, Expression, FunctionBody, Program, Statement};
use oxc_span::SPAN;
use oxc_traverse::{Traverse, TraverseCtx};

use crate::ast_deobfuscate::pass::counter_pass;
use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct SequenceExpressionSplitter {
    split_count: usize,
}

counter_pass!(SequenceExpressionSplitter, split_count, split_count);

impl<'a> Traverse<'a, DeobfuscateState> for SequenceExpressionSplitter {
    fn exit_program(&mut self, program: &mut Program<'a>, ctx: &mut Ctx<'a>) {
        let has_sequence = program.body.iter().any(|s| {
            matches!(
                s,
                Statement::ExpressionStatement(expr_stmt)
                    if matches!(expr_stmt.expression, Expression::SequenceExpression(_))
            )
        });
        if !has_sequence {
            return;
        }

        let before = program.body.len();
        let mut new_body = OxcVec::new_in(ctx);
        for stmt in &program.body {
            if let Statement::ExpressionStatement(expr_stmt) = stmt
                && let Expression::SequenceExpression(seq) = &expr_stmt.expression
            {
                eprintln!(
                    "[SEQ_SPLIT] Splitting sequence with {} expressions in program body",
                    seq.expressions.len()
                );
                for expr in &seq.expressions {
                    new_body.push(Statement::new_expression_statement(
                        SPAN,
                        expr.clone_in_with_semantic_ids(ctx.allocator()),
                        &ctx.ast,
                    ));
                }
                self.split_count += 1;
                continue;
            }
            new_body.push(stmt.clone_in_with_semantic_ids(ctx.allocator()));
        }
        let after = new_body.len();
        eprintln!("[SEQ_SPLIT] Program body: {before} -> {after} statements");
        program.body = new_body;
    }

    fn exit_block_statement(&mut self, block: &mut BlockStatement<'a>, ctx: &mut Ctx<'a>) {
        let has_sequence = block.body.iter().any(|s| {
            matches!(
                s,
                Statement::ExpressionStatement(expr_stmt)
                    if matches!(expr_stmt.expression, Expression::SequenceExpression(_))
            )
        });
        if !has_sequence {
            return;
        }

        let before = block.body.len();
        let mut new_body = OxcVec::new_in(ctx);
        for stmt in &block.body {
            if let Statement::ExpressionStatement(expr_stmt) = stmt
                && let Expression::SequenceExpression(seq) = &expr_stmt.expression
            {
                eprintln!(
                    "[SEQ_SPLIT] Splitting sequence with {} expressions in block",
                    seq.expressions.len()
                );
                for expr in &seq.expressions {
                    new_body.push(Statement::new_expression_statement(
                        SPAN,
                        expr.clone_in_with_semantic_ids(ctx.allocator()),
                        &ctx.ast,
                    ));
                }
                self.split_count += 1;
                continue;
            }
            new_body.push(stmt.clone_in_with_semantic_ids(ctx.allocator()));
        }
        let after = new_body.len();
        eprintln!("[SEQ_SPLIT] Block body: {before} -> {after} statements");
        block.body = new_body;
    }

    fn exit_function_body(&mut self, body: &mut FunctionBody<'a>, ctx: &mut Ctx<'a>) {
        let has_sequence = body.statements.iter().any(|s| {
            matches!(
                s,
                Statement::ExpressionStatement(expr_stmt)
                    if matches!(expr_stmt.expression, Expression::SequenceExpression(_))
            )
        });
        if !has_sequence {
            return;
        }

        let before = body.statements.len();
        let mut new_stmts = OxcVec::new_in(ctx);
        for stmt in &body.statements {
            if let Statement::ExpressionStatement(expr_stmt) = stmt
                && let Expression::SequenceExpression(seq) = &expr_stmt.expression
            {
                eprintln!(
                    "[SEQ_SPLIT] Splitting sequence with {} expressions in function body",
                    seq.expressions.len()
                );
                for expr in &seq.expressions {
                    new_stmts.push(Statement::new_expression_statement(
                        SPAN,
                        expr.clone_in_with_semantic_ids(ctx.allocator()),
                        &ctx.ast,
                    ));
                }
                self.split_count += 1;
                continue;
            }
            new_stmts.push(stmt.clone_in_with_semantic_ids(ctx.allocator()));
        }
        let after = new_stmts.len();
        eprintln!("[SEQ_SPLIT] Function body: {before} -> {after} statements");
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

    fn run_split(code: &str) -> (String, usize) {
        let allocator = Allocator::default();
        let source_type = SourceType::mjs();
        let ret = Parser::new(&allocator, code, source_type).parse();
        let mut program = ret.program;

        let mut splitter = SequenceExpressionSplitter::new();
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

        traverse_mut_with_ctx(&mut splitter, &mut program, &mut ctx);

        (Codegen::new().build(&program).code, splitter.split_count())
    }

    #[test]
    fn test_split_simple_sequence() {
        let (output, count) = run_split("a = 1, b = 2, c = 3;");
        assert!(count >= 1, "Should have split at least 1 sequence, got: {count}");
        assert!(
            output.contains("a = 1;\n"),
            "Should have 'a = 1;' as separate statement, got: {output}"
        );
        assert!(
            output.contains("b = 2;\n"),
            "Should have 'b = 2;' as separate statement, got: {output}"
        );
        assert!(
            output.contains("c = 3;\n"),
            "Should have 'c = 3;' as separate statement, got: {output}"
        );
    }

    #[test]
    fn test_no_split_for_init() {
        let (output, count) = run_split("for (a = 0, b = 1;;) {}");
        assert_eq!(count, 0, "Should NOT split sequence in for-init, got: {count}");
        assert!(output.contains("for"), "Should preserve for loop, got: {output}");
    }

    #[test]
    fn test_split_in_function_body() {
        let (output, count) = run_split("function f() { x = 1, y = 2; }");
        assert!(count >= 1, "Should have split sequence in function body, got: {count}");
        assert!(
            output.contains("x = 1;") && output.contains("y = 2;"),
            "Should have separate statements in function body, got: {output}"
        );
    }

    #[test]
    fn test_no_split_single_expression() {
        let (_output, count) = run_split("a = 1;");
        assert_eq!(count, 0, "Should not split single expression statement, got: {count}");
    }

    #[test]
    fn test_preserve_return_sequence() {
        let (output, count) = run_split("function f() { return a = 1, b; }");
        assert_eq!(count, 0, "Should NOT split sequence inside return, got: {count}");
        assert!(
            output.contains("return"),
            "Should preserve return statement, got: {output}"
        );
    }
}
