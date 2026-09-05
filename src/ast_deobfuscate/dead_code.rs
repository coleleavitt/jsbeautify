use oxc_allocator::CloneIn;
use oxc_ast::ast::{BlockStatement, Expression, Statement};
use oxc_semantic::ScopeFlags;
use oxc_span::SPAN;
use oxc_traverse::{Traverse, TraverseCtx};

use crate::ast_deobfuscate::pass::flag_pass;
use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct DeadCodeEliminator {
    changed: bool,
}

impl DeadCodeEliminator {
    fn is_false(expr: &Expression<'_>) -> bool {
        match expr {
            Expression::BooleanLiteral(lit) => !lit.value,
            Expression::NumericLiteral(lit) => lit.value == 0.0,
            _ => false,
        }
    }

    fn is_true(expr: &Expression<'_>) -> bool {
        match expr {
            Expression::BooleanLiteral(lit) => lit.value,
            Expression::NumericLiteral(lit) => lit.value != 0.0,
            _ => false,
        }
    }

    fn clone_statement<'b>(stmt: &Statement<'b>, ctx: &mut TraverseCtx<'b, DeobfuscateState>) -> Statement<'b> {
        stmt.clone_in_with_semantic_ids(ctx)
    }
}

flag_pass!(DeadCodeEliminator, changed, has_changed);

impl<'a> Traverse<'a, DeobfuscateState> for DeadCodeEliminator {
    fn exit_statement(&mut self, stmt: &mut Statement<'a>, ctx: &mut Ctx<'a>) {
        match stmt {
            Statement::IfStatement(if_stmt) => {
                if Self::is_false(&if_stmt.test) {
                    eprintln!("[AST] Eliminating if(false) branch");
                    self.changed = true;
                    *stmt = Statement::new_empty_statement(SPAN, ctx);
                } else if Self::is_true(&if_stmt.test) {
                    eprintln!("[AST] Eliminating if(true) - keeping consequent");
                    self.changed = true;
                    let scope_id = ctx.create_child_scope_of_current(ScopeFlags::empty());
                    *stmt = ctx.ast.statement_block_with_scope_id(SPAN, ctx.ast.vec(), scope_id);
                }
            }
            Statement::WhileStatement(while_stmt) if Self::is_false(&while_stmt.test) => {
                eprintln!("[AST] Eliminating while(false) loop");
                self.changed = true;
                *stmt = Statement::new_empty_statement(SPAN, ctx);
            }
            _ => {}
        }
    }

    fn exit_block_statement(&mut self, block: &mut BlockStatement<'a>, ctx: &mut Ctx<'a>) {
        let mut has_terminator = false;
        let mut new_body = ctx.ast.vec();

        for stmt in &block.body {
            if has_terminator {
                eprintln!("[AST] Removing unreachable code after return/break/continue/throw");
                self.changed = true;
                continue;
            }

            let is_terminator = matches!(
                stmt,
                Statement::ReturnStatement(_)
                    | Statement::BreakStatement(_)
                    | Statement::ContinueStatement(_)
                    | Statement::ThrowStatement(_)
            );

            new_body.push(Self::clone_statement(stmt, ctx));

            if is_terminator {
                has_terminator = true;
            }
        }

        if self.changed {
            block.body = new_body;
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

    fn run_dce(code: &str) -> String {
        let allocator = Allocator::default();
        let source_type = SourceType::mjs();
        let ret = Parser::new(&allocator, code, source_type).parse();
        let mut program = ret.program;

        let mut eliminator = DeadCodeEliminator::new();
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

        traverse_mut_with_ctx(&mut eliminator, &mut program, &mut ctx);

        Codegen::new().build(&program).code
    }

    #[test]
    fn test_remove_if_false() {
        let output = run_dce("if (false) { console.log('dead'); }");
        assert!(
            !output.contains("dead"),
            "Should remove if(false) branch, got: {output}"
        );
    }

    #[test]
    fn test_remove_while_false() {
        let output = run_dce("while (false) { console.log('dead'); }");
        assert!(
            !output.contains("while"),
            "Should remove while(false) loop, got: {output}"
        );
    }

    #[test]
    fn test_preserve_reachable_code() {
        let output = run_dce("var x = 1; console.log(x);");
        assert!(
            output.contains("console"),
            "Should preserve reachable code, got: {output}"
        );
    }
}
