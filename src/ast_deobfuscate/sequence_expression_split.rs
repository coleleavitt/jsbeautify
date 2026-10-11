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
use oxc_ast::ast::{BlockStatement, Expression, FunctionBody, Program, Statement, SwitchCase};
use oxc_span::SPAN;
use oxc_traverse::{Traverse, TraverseCtx};

use crate::ast_deobfuscate::pass::counter_pass;
use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct SequenceExpressionSplitter {
    split_count: usize,
}

counter_pass!(SequenceExpressionSplitter, split_count, split_count);

pub fn is_sequence_stmt(stmt: &Statement<'_>) -> bool {
    matches!(
        stmt,
        Statement::ExpressionStatement(expr_stmt) if matches!(expr_stmt.expression, Expression::SequenceExpression(_))
    )
}

impl SequenceExpressionSplitter {
    fn split_statements<'a>(&mut self, stmts: &mut OxcVec<'a, Statement<'a>>, ctx: &mut Ctx<'a>) {
        if !stmts.iter().any(is_sequence_stmt) {
            return;
        }

        let mut new_stmts = OxcVec::new_in(ctx);
        for stmt in stmts.iter() {
            if let Statement::ExpressionStatement(expr_stmt) = stmt
                && let Expression::SequenceExpression(seq) = &expr_stmt.expression
            {
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
        *stmts = new_stmts;
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for SequenceExpressionSplitter {
    fn exit_program(&mut self, program: &mut Program<'a>, ctx: &mut Ctx<'a>) {
        self.split_statements(&mut program.body, ctx);
    }

    fn exit_block_statement(&mut self, block: &mut BlockStatement<'a>, ctx: &mut Ctx<'a>) {
        self.split_statements(&mut block.body, ctx);
    }

    fn exit_function_body(&mut self, body: &mut FunctionBody<'a>, ctx: &mut Ctx<'a>) {
        self.split_statements(&mut body.statements, ctx);
    }

    fn exit_switch_case(&mut self, case: &mut SwitchCase<'a>, ctx: &mut Ctx<'a>) {
        self.split_statements(&mut case.consequent, ctx);
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

    #[test]
    fn splits_in_switch_case() {
        let (output, count) = run_split("switch (k) { case 1: a(), b(); }");
        assert_eq!(count, 1);
        assert!(output.contains("a();\n") && output.contains("b();\n"), "got: {output}");
    }
}
