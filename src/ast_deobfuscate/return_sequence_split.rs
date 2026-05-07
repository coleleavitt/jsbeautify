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

use oxc_allocator::CloneIn;
use oxc_ast::ast::{Expression, FunctionBody, Statement};
use oxc_span::SPAN;
use oxc_traverse::{Traverse, TraverseCtx};

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct ReturnSequenceSplitter {
    split_count: usize,
}

impl ReturnSequenceSplitter {
    #[must_use]
    pub const fn new() -> Self {
        Self { split_count: 0 }
    }

    #[must_use]
    pub const fn split_count(&self) -> usize {
        self.split_count
    }
}

impl Default for ReturnSequenceSplitter {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for ReturnSequenceSplitter {
    fn exit_function_body(&mut self, body: &mut FunctionBody<'a>, ctx: &mut Ctx<'a>) {
        self.split_in_stmts(&mut body.statements, ctx);
    }

    fn exit_block_statement(&mut self, block: &mut oxc_ast::ast::BlockStatement<'a>, ctx: &mut Ctx<'a>) {
        self.split_in_stmts(&mut block.body, ctx);
    }
}

impl ReturnSequenceSplitter {
    fn split_in_stmts<'a>(&mut self, stmts: &mut oxc_allocator::Vec<'a, Statement<'a>>, ctx: &mut Ctx<'a>) {
        let has_return_seq = stmts.iter().any(|s| {
            if let Statement::ReturnStatement(ret) = s {
                if let Some(Expression::SequenceExpression(seq)) = &ret.argument {
                    return seq.expressions.len() >= 2;
                }
            }
            false
        });

        if !has_return_seq {
            return;
        }

        let mut new_stmts = ctx.ast.vec();
        for stmt in stmts.iter() {
            if let Statement::ReturnStatement(ret) = stmt {
                if let Some(Expression::SequenceExpression(seq)) = &ret.argument {
                    if seq.expressions.len() >= 2 {
                        // Emit all but the last expression as expression-statements.
                        let len = seq.expressions.len();
                        for expr in &seq.expressions[..len - 1] {
                            new_stmts.push(
                                ctx.ast
                                    .statement_expression(SPAN, expr.clone_in_with_semantic_ids(ctx.ast.allocator)),
                            );
                        }
                        // Emit `return <last>;`
                        let last = &seq.expressions[len - 1];
                        let ret_stmt = ctx
                            .ast
                            .statement_return(SPAN, Some(last.clone_in_with_semantic_ids(ctx.ast.allocator)));
                        new_stmts.push(ret_stmt);
                        self.split_count += 1;
                        continue;
                    }
                }
            }
            new_stmts.push(stmt.clone_in_with_semantic_ids(ctx.ast.allocator));
        }
        *stmts = new_stmts;
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
}
