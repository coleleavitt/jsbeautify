//! Return-sequence splitting pass
//!
//! Splits sequence expressions (comma operator) inside `return` statements
//! into preceding expression-statements followed by a `return` of the last value.
//!
//! Before:
//! ```js
//! return foo.set(H, $), $;
//! return this.size += q.size == K ? 0 : 1, this;
//! ```
//!
//! After:
//! ```js
//! foo.set(H, $);
//! return $;
//!
//! this.size += q.size == K ? 0 : 1;
//! return this;
//! ```
//!
//! This is safe because JavaScript's comma operator evaluates left-to-right
//! and discards all but the last value. Splitting preserves evaluation order.
//!
//! Only fires when the `return` is inside a block (function body / if / else / etc.)
//! where we can insert preceding statements.

use oxc_allocator::{ArenaVec, TakeIn};
use oxc_ast::ast::{Expression, FunctionBody, Statement, SwitchCase};
use oxc_ast::builder::AstBuilder;
use oxc_span::SPAN;
use oxc_traverse::{Traverse, TraverseCtx};

use crate::ast_deobfuscate::pass::counter_pass;
use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct ReturnSequenceSplitter {
    split_count: usize,
}

counter_pass!(ReturnSequenceSplitter, split_count, split_count);

impl<'a> Traverse<'a, DeobfuscateState> for ReturnSequenceSplitter {
    fn exit_function_body(&mut self, body: &mut FunctionBody<'a>, ctx: &mut Ctx<'a>) {
        self.split_in_stmts(&mut body.statements, ctx);
    }

    fn exit_block_statement(&mut self, block: &mut oxc_ast::ast::BlockStatement<'a>, ctx: &mut Ctx<'a>) {
        self.split_in_stmts(&mut block.body, ctx);
    }

    fn exit_switch_case(&mut self, case: &mut SwitchCase<'a>, ctx: &mut Ctx<'a>) {
        self.split_in_stmts(&mut case.consequent, ctx);
    }
}

pub fn is_return_sequence(stmt: &Statement<'_>) -> bool {
    matches!(
        stmt,
        Statement::ReturnStatement(ret)
            if matches!(&ret.argument, Some(Expression::SequenceExpression(seq)) if seq.expressions.len() >= 2)
    )
}

/// Splits `return a, b;` into `a; return b;`, or gives `stmt` back when it
/// is not a return of a sequence.
pub fn split_return_sequence<'a>(
    stmt: Statement<'a>,
    ast: &AstBuilder<'a>,
) -> Result<ArenaVec<'a, Statement<'a>>, Statement<'a>> {
    if !is_return_sequence(&stmt) {
        return Err(stmt);
    }
    let Statement::ReturnStatement(ret) = stmt else {
        unreachable!("is_return_sequence matched")
    };
    let Some(Expression::SequenceExpression(seq)) = ret.unbox().argument else {
        unreachable!("is_return_sequence matched")
    };
    let mut expressions = seq.unbox().expressions;
    let last = expressions.pop().expect("is_return_sequence checked two expressions");
    let mut stmts = ArenaVec::from_iter_in(
        expressions
            .into_iter()
            .map(|expr| Statement::new_expression_statement(SPAN, expr, ast)),
        ast,
    );
    stmts.push(Statement::new_return_statement(SPAN, Some(last), ast));
    Ok(stmts)
}

impl ReturnSequenceSplitter {
    fn split_in_stmts<'a>(&mut self, stmts: &mut ArenaVec<'a, Statement<'a>>, ctx: &mut Ctx<'a>) {
        if !stmts.iter().any(is_return_sequence) {
            return;
        }
        for stmt in stmts.take_in(ctx) {
            match split_return_sequence(stmt, &ctx.ast) {
                Ok(parts) => {
                    stmts.extend(parts);
                    self.split_count += 1;
                }
                Err(stmt) => stmts.push(stmt),
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

    fn run(code: &str) -> (String, usize) {
        let allocator = Allocator::default();
        let source_type = SourceType::mjs();
        let ret = Parser::new(&allocator, code, source_type).parse();
        let mut program = ret.program;

        let mut splitter = ReturnSequenceSplitter::new();
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

        traverse_mut_with_ctx(&mut splitter, &mut program, &mut ctx);

        (Codegen::new().build(&program).code, splitter.split_count())
    }

    #[test]
    fn splits_simple_return_sequence() {
        let (out, count) = run("function f() { return foo.set(a, b), b; }");
        assert_eq!(count, 1);
        assert!(out.contains("foo.set(a, b);"), "Should have expression stmt: {out}");
        assert!(out.contains("return b;"), "Should have return b: {out}");
    }

    #[test]
    fn splits_multi_expression_sequence() {
        let (out, count) = run("function f() { return a(), b(), c; }");
        assert_eq!(count, 1);
        assert!(out.contains("a();"), "Should have a(): {out}");
        assert!(out.contains("b();"), "Should have b(): {out}");
        assert!(out.contains("return c;"), "Should have return c: {out}");
    }

    #[test]
    fn preserves_non_sequence_return() {
        let (out, count) = run("function f() { return 42; }");
        assert_eq!(count, 0);
        assert!(out.contains("return 42;"), "Should preserve: {out}");
    }

    #[test]
    fn preserves_single_element_sequence() {
        let (out, count) = run("function f() { return (a); }");
        assert_eq!(count, 0);
        assert!(out.contains("return"), "Should preserve return: {out}");
    }

    #[test]
    fn splits_in_switch_case() {
        let (out, count) = run("function f() { switch (k) { case 1: return a(), b; } }");
        assert_eq!(count, 1);
        assert!(out.contains("a();") && out.contains("return b;"), "got: {out}");
    }
}
