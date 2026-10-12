//! Wraps a single-statement `if`/`else` or loop body in a block when the
//! statement-split passes would expand it into several statements or an `if`:
//!
//! ```js
//! if (x) a = b, c();
//! while (y) z && w();
//! ```
//!
//! Becomes:
//!
//! ```js
//! if (x) { a = b, c(); }
//! while (y) { z && w(); }
//! ```
//!
//! The splitting itself is left to the passes that own each shape
//! (`sequence_expression_split`, `short_circuit_to_if`, `ternary_to_if_else`,
//! `return_sequence_split`), which only rewrite statement lists.

use oxc_allocator::{ArenaVec, TakeIn};
use oxc_ast::ast::{
    DoWhileStatement, ForInStatement, ForOfStatement, ForStatement, IfStatement, Statement, WhileStatement,
};
use oxc_semantic::ScopeFlags;
use oxc_span::SPAN;
use oxc_traverse::{Traverse, TraverseCtx};

use crate::ast_deobfuscate::pass::counter_pass;
use crate::ast_deobfuscate::return_sequence_split::is_return_sequence;
use crate::ast_deobfuscate::sequence_expression_split::is_sequence_stmt;
use crate::ast_deobfuscate::short_circuit_to_if::is_short_circuit_expr_stmt;
use crate::ast_deobfuscate::state::DeobfuscateState;
use crate::ast_deobfuscate::ternary_to_if_else::is_convertible;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct BodyBlockWrapper {
    wrapped_count: usize,
}

counter_pass!(BodyBlockWrapper, wrapped_count, wrapped_count);

impl BodyBlockWrapper {
    fn wrap<'a>(&mut self, body: &mut Statement<'a>, ctx: &mut Ctx<'a>) {
        if !(is_sequence_stmt(body)
            || is_short_circuit_expr_stmt(body)
            || is_convertible(body)
            || is_return_sequence(body))
        {
            return;
        }
        let mut stmts = ArenaVec::new_in(&ctx.ast);
        stmts.push(body.take_in(ctx));
        let scope_id = ctx.create_child_scope_of_current(ScopeFlags::empty());
        *body = Statement::new_block_statement_with_scope_id(SPAN, stmts, scope_id, &ctx.ast);
        self.wrapped_count += 1;
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for BodyBlockWrapper {
    fn exit_if_statement(&mut self, stmt: &mut IfStatement<'a>, ctx: &mut Ctx<'a>) {
        self.wrap(&mut stmt.consequent, ctx);
        if let Some(alternate) = &mut stmt.alternate {
            self.wrap(alternate, ctx);
        }
    }

    fn exit_while_statement(&mut self, stmt: &mut WhileStatement<'a>, ctx: &mut Ctx<'a>) {
        self.wrap(&mut stmt.body, ctx);
    }

    fn exit_do_while_statement(&mut self, stmt: &mut DoWhileStatement<'a>, ctx: &mut Ctx<'a>) {
        self.wrap(&mut stmt.body, ctx);
    }

    fn exit_for_statement(&mut self, stmt: &mut ForStatement<'a>, ctx: &mut Ctx<'a>) {
        self.wrap(&mut stmt.body, ctx);
    }

    fn exit_for_in_statement(&mut self, stmt: &mut ForInStatement<'a>, ctx: &mut Ctx<'a>) {
        self.wrap(&mut stmt.body, ctx);
    }

    fn exit_for_of_statement(&mut self, stmt: &mut ForOfStatement<'a>, ctx: &mut Ctx<'a>) {
        self.wrap(&mut stmt.body, ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast_deobfuscate::semantic;
    use oxc_allocator::Allocator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_span::SourceType;
    use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

    fn run(code: &str) -> (String, usize) {
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let mut program = ret.program;
        let mut wrapper = BodyBlockWrapper::new();
        let scoping = semantic::scoping(&program);
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        traverse_mut_with_ctx(&mut wrapper, &mut program, &mut ctx);
        (Codegen::new().build(&program).code, wrapper.wrapped_count())
    }

    #[test]
    fn wraps_if_and_else_bodies() {
        let (out, count) = run("if (x) a = 1, b(); else c && d();");
        assert_eq!(count, 2);
        assert_eq!(out, "if (x) {\n\ta = 1, b();\n} else {\n\tc && d();\n}\n");
    }

    #[test]
    fn wraps_loop_bodies() {
        let (_, count) = run(
            "while (x) a(), b(); do c ? d() : e(); while (y); for (;;) f || g(); for (k in o) h(), i(); for (v of l) j && m();",
        );
        assert_eq!(count, 5);
    }

    #[test]
    fn wraps_return_sequence_body() {
        let (out, count) = run("function f() { if (x) return a(), b; }");
        assert_eq!(count, 1);
        assert!(out.contains("if (x) {\n\t\treturn a(), b;\n\t}"), "got: {out}");
    }

    #[test]
    fn leaves_bodies_no_pass_expands() {
        let (out, count) =
            run("if (x) foo(); else if (y) bar(); while (z) var a = 1, b = 2; for (;;) return c ? d : e;");
        assert_eq!(count, 0, "got: {out}");
    }
}
