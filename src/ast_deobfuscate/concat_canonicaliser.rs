//! String-coercion canonicaliser.
//!
//! Obfuscators (and TypeScript targeting ES5) frequently rewrite `a + "=" + b`
//! as `"".concat(a, "=", b)` to hide the concatenation operator. This pass
//! detects any call where the receiver is a string literal or a chain of `.concat()`
//! calls and rewrites it as a plain binary `+` chain:
//!
//! ```text
//! "".concat(a, "=", b)   ->  "" + a + "=" + b   ->  a + "=" + b (after const-folding)
//! "prefix".concat(x)     ->  "prefix" + x
//! a.concat(b)            ->  (left alone if `a` is not provably a string)
//! ```
//!
//! The pass is conservative: it only fires when the receiver is a string literal,
//! or when the receiver is itself a `.concat` call (nested concat chains collapse).

use oxc_allocator::CloneIn;
use oxc_ast::ast::{BinaryExpression, BinaryOperator, Expression};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use oxc_traverse::{Traverse, TraverseCtx};
use std::cell::Cell;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct ConcatCanonicaliser {
    rewrites: usize,
}

impl ConcatCanonicaliser {
    #[must_use]
    pub const fn new() -> Self {
        Self { rewrites: 0 }
    }

    #[must_use]
    pub const fn rewrites(&self) -> usize {
        self.rewrites
    }
}

impl Default for ConcatCanonicaliser {
    fn default() -> Self {
        Self::new()
    }
}

fn receiver_is_string_like(expr: &Expression<'_>) -> bool {
    match expr {
        Expression::StringLiteral(_) => true,
        Expression::ParenthesizedExpression(p) => receiver_is_string_like(&p.expression),
        Expression::BinaryExpression(b) if matches!(b.operator, BinaryOperator::Addition) => {
            receiver_is_string_like(&b.left) || receiver_is_string_like(&b.right)
        }
        Expression::CallExpression(call) => {
            if let Expression::StaticMemberExpression(sme) = &call.callee
                && sme.property.name.as_str() == "concat"
            {
                receiver_is_string_like(&sme.object)
            } else {
                false
            }
        }
        _ => false,
    }
}

fn make_plus<'a>(left: Expression<'a>, right: Expression<'a>, ctx: &Ctx<'a>) -> Expression<'a> {
    Expression::BinaryExpression(ctx.ast.alloc(BinaryExpression {
        node_id: Cell::new(NodeId::DUMMY),
        span: SPAN,
        left,
        operator: BinaryOperator::Addition,
        right,
    }))
}

impl<'a> Traverse<'a, DeobfuscateState> for ConcatCanonicaliser {
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        let Expression::CallExpression(call) = expr else {
            return;
        };
        let Expression::StaticMemberExpression(sme) = &call.callee else {
            return;
        };
        if sme.property.name.as_str() != "concat" {
            return;
        }
        if !receiver_is_string_like(&sme.object) {
            return;
        }
        if call.arguments.iter().any(|a| a.as_expression().is_none()) {
            return;
        }
        if self.rewrites < 10 {
            eprintln!(
                "[AST/concat] rewriting <str>.concat({} args) -> binary '+'",
                call.arguments.len()
            );
        }
        self.rewrites += 1;
        let mut acc = sme.object.clone_in_with_semantic_ids(ctx.ast.allocator);
        for arg in &call.arguments {
            let rhs = arg
                .as_expression()
                .unwrap()
                .clone_in_with_semantic_ids(ctx.ast.allocator);
            acc = make_plus(acc, rhs, ctx);
        }
        *expr = acc;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_allocator::Allocator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_semantic::SemanticBuilder;
    use oxc_span::SourceType;
    use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

    fn run(code: &str) -> String {
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let mut program = ret.program;
        let mut pass = ConcatCanonicaliser::new();
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);
        traverse_mut_with_ctx(&mut pass, &mut program, &mut ctx);
        Codegen::new().build(&program).code
    }

    #[test]
    fn empty_string_concat() {
        let out = run("var x = \"\".concat(a, \"=\", b);");
        assert!(!out.contains(".concat"), "got: {out}");
        assert!(out.contains('+'), "must use +, got: {out}");
    }

    #[test]
    fn prefix_concat() {
        let out = run("var x = \"prefix\".concat(y);");
        assert!(!out.contains(".concat"), "got: {out}");
        assert!(out.contains("\"prefix\""));
        assert!(out.contains("y"));
    }

    #[test]
    fn leaves_non_string_receiver_alone() {
        let out = run("var x = arr.concat(y, z);");
        assert!(out.contains(".concat"), "non-string receiver kept: {out}");
    }

    #[test]
    fn collapses_nested_concat() {
        let out = run("var x = \"\".concat(a).concat(b);");
        assert!(!out.contains(".concat"), "got: {out}");
    }
}
