//! Empty statement cleanup pass
//!
//! Removes `EmptyStatement` (`;`) nodes from program body and block bodies.
//! These are left behind by other deobfuscation passes that replace removed
//! statements (proxy functions, dead code, etc.) with `EmptyStatement`.

use oxc_allocator::ArenaVec;
use oxc_ast::ast::{BlockStatement, FunctionBody, Program, Statement};
use oxc_traverse::{Traverse, TraverseCtx};

use crate::ast_deobfuscate::pass::counter_pass;
use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct EmptyStatementCleanup {
    removed_count: usize,
}

counter_pass!(EmptyStatementCleanup, removed_count, removed_count);

impl EmptyStatementCleanup {
    /// Drops `;` statements from `body` in place. The kept statements are moved,
    /// not cloned, so the cost is linear in `body.len()` and independent of how
    /// deep the subtrees below them are.
    fn remove_empty(&mut self, body: &mut ArenaVec<'_, Statement<'_>>, kind: &str) {
        let before = body.len();
        body.retain(|stmt| !matches!(stmt, Statement::EmptyStatement(_)));
        let removed = before - body.len();
        if removed > 0 {
            self.removed_count += removed;
            deob_trace!(
                "[EMPTY_CLEANUP] Removed {removed} empty statements from {kind} ({before} -> {})",
                body.len()
            );
        }
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for EmptyStatementCleanup {
    fn exit_program(&mut self, program: &mut Program<'a>, _ctx: &mut Ctx<'a>) {
        self.remove_empty(&mut program.body, "program body");
    }

    fn exit_block_statement(&mut self, block: &mut BlockStatement<'a>, _ctx: &mut Ctx<'a>) {
        self.remove_empty(&mut block.body, "block");
    }

    fn exit_function_body(&mut self, body: &mut FunctionBody<'a>, _ctx: &mut Ctx<'a>) {
        self.remove_empty(&mut body.statements, "function body");
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
    fn removes_empty_statements_at_every_nesting_level() {
        let (output, count) =
            run_cleanup("function f() { ; return function g() { ; { ; } return function h() { ; return 1; }; }; }");
        assert_eq!(count, 4, "{output}");
        assert_eq!(
            output,
            "function f() {\n\treturn function g() {\n\t\t{}\n\t\treturn function h() {\n\t\t\treturn 1;\n\t\t};\n\t};\n}\n"
        );
    }

    #[test]
    fn keeps_empty_statements_that_are_a_whole_body() {
        let (output, count) = run_cleanup("if (a) ; else ; while (b) ; label: ;");
        assert_eq!(count, 0, "{output}");
        assert_eq!(output, "if (a);\nelse;\nwhile (b);\nlabel:;\n");
    }

    #[test]
    fn test_preserve_for_empty() {
        // for(;;) uses empty statements legitimately — but those are in ForStatement, not blocks
        let (output, _count) = run_cleanup("for (;;) { break; }");
        assert!(output.contains("for"), "Should preserve for loop, got: {output}");
    }
}
