use oxc_ast::ast::{CatchClause, Statement};
use oxc_span::SPAN;
use oxc_traverse::{Traverse, TraverseCtx};

use crate::ast_deobfuscate::pass::counter_pass;
use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct TryCatchRemover {
    rewrites: usize,
}

impl TryCatchRemover {
    fn is_catch_empty(handler: &CatchClause<'_>) -> bool {
        handler.body.body.is_empty()
    }
}

counter_pass!(TryCatchRemover, rewrites, rewrites);

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

            // An empty catch still swallows whatever the try body throws, so
            // only a statically empty try block can be dropped.
            if !try_stmt.block.body.is_empty() {
                return;
            }
            self.rewrites += 1;
            *stmt = Statement::new_empty_statement(SPAN, ctx);
        }
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

    fn run_try_catch_remover(code: &str) -> String {
        let allocator = Allocator::default();
        let source_type = SourceType::mjs();
        let ret = Parser::new(&allocator, code, source_type).parse();
        let mut program = ret.program;

        let mut remover = TryCatchRemover::new();
        let state = DeobfuscateState::new();
        let scoping = semantic::scoping(&program);
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

        traverse_mut_with_ctx(&mut remover, &mut program, &mut ctx);

        Codegen::new().build(&program).code
    }

    #[test]
    fn test_remove_empty_try_and_catch() {
        let output = run_try_catch_remover("try {} catch(e) {} f();");
        assert!(!output.contains("try"), "Should remove empty try, got: {output}");
        assert!(output.contains("f()"), "Should keep following code, got: {output}");
    }

    #[test]
    fn test_preserve_empty_catch_that_swallows_throw() {
        // lodash `getRawTag`: the assignment throws on frozen/read-only tags.
        let output = run_try_catch_remover("try { t[k] = void 0; var n = true; } catch(e) {}");
        assert!(
            output.contains("try"),
            "Empty catch still swallows throws, got: {output}"
        );
        assert!(
            output.contains("catch"),
            "Empty catch still swallows throws, got: {output}"
        );
    }

    #[test]
    fn test_preserve_empty_catch_around_throw() {
        let output = run_try_catch_remover("try { throw 0; } catch {}");
        assert!(
            output.contains("catch"),
            "Empty catch swallows the throw, got: {output}"
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
