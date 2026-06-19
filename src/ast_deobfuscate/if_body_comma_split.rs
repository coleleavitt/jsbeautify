//! If-body comma expression splitter
//!
//! Splits sequence expressions used as the consequent/alternate of `if` statements
//! (without braces) into block statements with individual expression-statements.
//!
//! Before:
//! ```js
//! if (X) a = b, c(), d = e;
//! ```
//!
//! After:
//! ```js
//! if (X) {
//!     a = b;
//!     c();
//!     d = e;
//! }
//! ```
//!
//! Only applies when the consequent/alternate is an `ExpressionStatement` containing
//! a `SequenceExpression` (the comma operator in a non-braced if body).

use oxc_allocator::CloneIn;
use oxc_ast::ast::{Expression, Statement};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use oxc_traverse::{Traverse, TraverseCtx};
use std::cell::Cell;

use crate::ast_deobfuscate::pass::counter_pass;
use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct IfBodyCommaSplitter {
    split_count: usize,
}

counter_pass!(IfBodyCommaSplitter, split_count, split_count);

impl<'a> Traverse<'a, DeobfuscateState> for IfBodyCommaSplitter {
    fn exit_if_statement(&mut self, stmt: &mut oxc_ast::ast::IfStatement<'a>, ctx: &mut Ctx<'a>) {
        // Handle consequent
        if is_sequence_expr_stmt(&stmt.consequent) {
            let new_block = split_to_block(&stmt.consequent, ctx);
            stmt.consequent = new_block;
            self.split_count += 1;
        }

        // Handle alternate
        if let Some(alt) = &stmt.alternate
            && is_sequence_expr_stmt(alt)
        {
            let new_block = split_to_block(alt, ctx);
            stmt.alternate = Some(new_block);
            self.split_count += 1;
        }
    }
}

fn is_sequence_expr_stmt(stmt: &Statement<'_>) -> bool {
    if let Statement::ExpressionStatement(es) = stmt {
        matches!(es.expression, Expression::SequenceExpression(_))
    } else {
        false
    }
}

fn split_to_block<'a>(stmt: &Statement<'a>, ctx: &mut Ctx<'a>) -> Statement<'a> {
    let Statement::ExpressionStatement(es) = stmt else {
        return stmt.clone_in_with_semantic_ids(ctx.ast.allocator);
    };
    let Expression::SequenceExpression(seq) = &es.expression else {
        return stmt.clone_in_with_semantic_ids(ctx.ast.allocator);
    };

    let mut body = ctx.ast.vec();
    for expr in &seq.expressions {
        body.push(
            ctx.ast
                .statement_expression(SPAN, expr.clone_in_with_semantic_ids(ctx.ast.allocator)),
        );
    }

    Statement::BlockStatement(ctx.ast.alloc(oxc_ast::ast::BlockStatement {
        node_id: Cell::new(NodeId::DUMMY),
        span: SPAN,
        body,
        scope_id: Cell::new(None),
    }))
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
        let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let mut program = ret.program;

        let mut splitter = IfBodyCommaSplitter::new();
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);
        traverse_mut_with_ctx(&mut splitter, &mut program, &mut ctx);

        (Codegen::new().build(&program).code, splitter.split_count())
    }

    #[test]
    fn splits_if_body_comma() {
        let (out, count) = run("if (x) a = 1, b(), c = 2;");
        assert_eq!(count, 1);
        assert!(out.contains("a = 1;"), "Should split first expr: {out}");
        assert!(out.contains("b();"), "Should split second expr: {out}");
        assert!(out.contains("c = 2;"), "Should split third expr: {out}");
    }

    #[test]
    fn splits_else_body_comma() {
        let (out, count) = run("if (x) foo(); else a = 1, b = 2;");
        assert_eq!(count, 1);
        assert!(out.contains("a = 1;"), "Should split else body: {out}");
        assert!(out.contains("b = 2;"), "Should split else body: {out}");
    }

    #[test]
    fn preserves_non_sequence() {
        let (out, count) = run("if (x) foo();");
        assert_eq!(count, 0);
        assert!(out.contains("foo()"), "Should preserve: {out}");
    }
}
