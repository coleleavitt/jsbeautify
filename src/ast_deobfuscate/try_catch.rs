use oxc_allocator::Vec as OxcVec;
use oxc_ast::ast::{CatchClause, Statement};
use oxc_semantic::ScopeFlags;
use oxc_span::SPAN;
use oxc_traverse::{Traverse, TraverseCtx};

use crate::ast_deobfuscate::pass::flag_pass;
use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct TryCatchRemover {
    changed: bool,
}

impl TryCatchRemover {
    fn is_catch_empty(handler: &CatchClause<'_>) -> bool {
        handler.body.body.is_empty()
    }
}

flag_pass!(TryCatchRemover, changed, has_changed);

impl<'a> Traverse<'a, DeobfuscateState> for TryCatchRemover {
    fn exit_statement(&mut self, stmt: &mut Statement<'a>, ctx: &mut Ctx<'a>) {
        if let Statement::TryStatement(try_stmt) = stmt {
            if try_stmt.finalizer.is_some() {
                return;
            }

            let Some(handler) = &try_stmt.handler else {
                return;
            };

            if !Self::is_catch_empty(handler) {
                return;
            }

            eprintln!(
                "[TRY_CATCH] Removing empty try-catch, extracting try body ({} stmts)",
                try_stmt.block.body.len()
            );
            self.changed = true;

            let try_body_statements = std::mem::replace(&mut try_stmt.block.body, OxcVec::new_in(ctx));

            if try_body_statements.is_empty() {
                eprintln!("[TRY_CATCH] Empty try body -> EmptyStatement");
                *stmt = Statement::new_empty_statement(SPAN, ctx);
            } else if try_body_statements.len() == 1 {
                eprintln!("[TRY_CATCH] Single stmt try body -> unwrapping");
                let mut iter = try_body_statements.into_iter();
                if let Some(first) = iter.next() {
                    *stmt = first;
                } else {
                    eprintln!("[TRY_CATCH] ERROR: iterator was empty despite len() == 1");
                    *stmt = Statement::new_empty_statement(SPAN, ctx);
                }
            } else {
                let scope_id = ctx.create_child_scope_of_current(ScopeFlags::empty());
                eprintln!("[TRY_CATCH] Multi-stmt try body -> BlockStatement with scope_id: {scope_id:?}");
                *stmt = Statement::new_block_statement_with_scope_id(SPAN, try_body_statements, scope_id, ctx);
            }
        }
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

    fn run_try_catch_remover(code: &str) -> String {
        let allocator = Allocator::default();
        let source_type = SourceType::mjs();
        let ret = Parser::new(&allocator, code, source_type).parse();
        let mut program = ret.program;

        let mut remover = TryCatchRemover::new();
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

        traverse_mut_with_ctx(&mut remover, &mut program, &mut ctx);

        Codegen::new().build(&program).code
    }

    #[test]
    fn test_remove_empty_catch() {
        let output = run_try_catch_remover("try { var x = 1; } catch(e) {}");
        eprintln!("Output: {output}");
        assert!(!output.contains("try"), "Should remove try keyword, got: {output}");
        assert!(!output.contains("catch"), "Should remove catch keyword, got: {output}");
        assert!(
            output.contains("var x = 1") || output.contains("var x=1"),
            "Should keep try body, got: {output}"
        );
    }

    #[test]
    fn test_preserve_non_empty_catch() {
        let output = run_try_catch_remover("try { var x = 1; } catch(e) { console.log(e); }");
        eprintln!("Output: {output}");
        assert!(
            output.contains("try"),
            "Should keep try with non-empty catch, got: {output}"
        );
        assert!(output.contains("catch"), "Should keep non-empty catch, got: {output}");
    }

    #[test]
    fn test_preserve_finally() {
        let output = run_try_catch_remover("try { var x = 1; } catch(e) {} finally { cleanup(); }");
        eprintln!("Output: {output}");
        assert!(
            output.contains("try"),
            "Should preserve try with finally, got: {output}"
        );
    }
}
