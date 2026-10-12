//! Empty statement cleanup pass
//!
//! Removes `EmptyStatement` (`;`) nodes from program body and block bodies.
//! These are left behind by other deobfuscation passes that replace removed
//! statements (proxy functions, dead code, etc.) with `EmptyStatement`.

use oxc_allocator::{ArenaVec, CloneIn, GetAllocator};
use oxc_ast::ast::{BlockStatement, FunctionBody, Program, Statement};
use oxc_traverse::{Traverse, TraverseCtx};

use crate::ast_deobfuscate::pass::counter_pass;
use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct EmptyStatementCleanup {
    removed_count: usize,
}

counter_pass!(EmptyStatementCleanup, removed_count, removed_count);

impl<'a> Traverse<'a, DeobfuscateState> for EmptyStatementCleanup {
    fn exit_program(&mut self, program: &mut Program<'a>, ctx: &mut Ctx<'a>) {
        let before = program.body.len();
        let mut new_body = ArenaVec::new_in(ctx);
        for stmt in &program.body {
            if matches!(stmt, Statement::EmptyStatement(_)) {
                self.removed_count += 1;
            } else {
                new_body.push(stmt.clone_in_with_semantic_ids(ctx.allocator()));
            }
        }
        let after = new_body.len();
        if before != after {
            deob_trace!(
                "[EMPTY_CLEANUP] Removed {} empty statements from program body ({} -> {})",
                before - after,
                before,
                after
            );
            program.body = new_body;
        }
    }

    fn exit_block_statement(&mut self, block: &mut BlockStatement<'a>, ctx: &mut Ctx<'a>) {
        let has_empty = block.body.iter().any(|s| matches!(s, Statement::EmptyStatement(_)));
        if !has_empty {
            return;
        }

        let before = block.body.len();
        let mut new_body = ArenaVec::new_in(ctx);
        for stmt in &block.body {
            if matches!(stmt, Statement::EmptyStatement(_)) {
                self.removed_count += 1;
            } else {
                new_body.push(stmt.clone_in_with_semantic_ids(ctx.allocator()));
            }
        }
        let after = new_body.len();
        deob_trace!(
            "[EMPTY_CLEANUP] Removed {} empty statements from block ({} -> {})",
            before - after,
            before,
            after
        );
        block.body = new_body;
    }

    fn exit_function_body(&mut self, body: &mut FunctionBody<'a>, ctx: &mut Ctx<'a>) {
        let has_empty = body
            .statements
            .iter()
            .any(|s| matches!(s, Statement::EmptyStatement(_)));
        if !has_empty {
            return;
        }

        let before = body.statements.len();
        let mut new_stmts = ArenaVec::new_in(ctx);
        for stmt in &body.statements {
            if matches!(stmt, Statement::EmptyStatement(_)) {
                self.removed_count += 1;
            } else {
                new_stmts.push(stmt.clone_in_with_semantic_ids(ctx.allocator()));
            }
        }
        let after = new_stmts.len();
        deob_trace!(
            "[EMPTY_CLEANUP] Removed {} empty statements from function body ({} -> {})",
            before - after,
            before,
            after
        );
        body.statements = new_stmts;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast_deobfuscate::{DeobfuscateState, semantic};
    use oxc_allocator::Allocator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_span::SourceType;
    use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

    fn run_cleanup(code: &str) -> (String, usize) {
        let allocator = Allocator::default();
        let source_type = SourceType::mjs();
        let ret = Parser::new(&allocator, code, source_type).parse();
        let mut program = ret.program;

        let mut cleanup = EmptyStatementCleanup::new();
        let state = DeobfuscateState::new();
        let scoping = semantic::scoping(&program);
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

        traverse_mut_with_ctx(&mut cleanup, &mut program, &mut ctx);

        (Codegen::new().build(&program).code, cleanup.removed_count())
    }

    #[test]
    fn test_remove_empty_from_program() {
        let (output, count) = run_cleanup("var x = 1; ; ; var y = 2;");
        assert!(
            !output.contains("\n;\n"),
            "Should remove standalone semicolons, got: {output}"
        );
        assert!(count >= 2, "Should have removed at least 2, got: {count}");
    }

    #[test]
    fn test_remove_empty_from_block() {
        let (output, count) = run_cleanup("{ var x = 1; ; var y = 2; }");
        assert!(count >= 1, "Should have removed at least 1, got: {count}");
        assert!(
            output.contains('x') && output.contains('y'),
            "Should preserve non-empty statements, got: {output}"
        );
    }

    #[test]
    fn test_remove_empty_from_function_body() {
        let (output, count) = run_cleanup("function foo() { var x = 1; ; ; var y = 2; }");
        assert!(
            count >= 2,
            "Should have removed at least 2 from function body, got: {count}"
        );
        assert!(
            output.contains('x') && output.contains('y'),
            "Should preserve non-empty statements, got: {output}"
        );
    }

    #[test]
    fn test_preserve_for_empty() {
        // for(;;) uses empty statements legitimately — but those are in ForStatement, not blocks
        let (output, _count) = run_cleanup("for (;;) { break; }");
        assert!(output.contains("for"), "Should preserve for loop, got: {output}");
    }
}
