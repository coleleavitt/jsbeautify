//! `.apply(null, [...])` / `.call(null, ...)` → direct call simplifier.
//!
//! Rewrites:
//! - `fn.apply(null, [a, b, c])`       → `fn(a, b, c)`
//! - `fn.apply(undefined, [a, b, c])`  → `fn(a, b, c)`
//! - `fn.call(null, a, b, c)`          → `fn(a, b, c)`
//! - `fn.call(undefined, a, b, c)`     → `fn(a, b, c)`
//!
//! ONLY fires when:
//! 1. The receiver is a plain identifier or StaticMemberExpression (no dynamic callee — the
//!    rewrite would change `this` binding for `obj.method.apply(null, ...)`).
//! 2. For `apply`, the 2nd argument is a literal ArrayExpression with no spread elements.
//! 3. The first argument is the literal `null` or the identifier `undefined`.
//!
//! We also avoid rewriting `foo.apply.apply(null, ...)` chains — these are rare enough to
//! leave alone.

use oxc_allocator::{CloneIn, Vec as OxcVec};
use oxc_ast::ast::{Argument, ArrayExpressionElement, CallExpression, Expression};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use oxc_traverse::{Traverse, TraverseCtx};
use std::cell::Cell;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct ApplyCallSimplifier {
    rewrites: usize,
}

impl ApplyCallSimplifier {
    #[must_use]
    pub const fn new() -> Self {
        Self { rewrites: 0 }
    }

    #[must_use]
    pub const fn rewrites(&self) -> usize {
        self.rewrites
    }
}

impl Default for ApplyCallSimplifier {
    fn default() -> Self {
        Self::new()
    }
}

fn is_null_or_undefined(expr: &Expression<'_>) -> bool {
    match expr {
        Expression::NullLiteral(_) => true,
        Expression::Identifier(id) => id.name.as_str() == "undefined",
        Expression::UnaryExpression(u) if matches!(u.operator, oxc_ast::ast::UnaryOperator::Void) => true,
        _ => false,
    }
}

fn callee_is_safe_receiver(expr: &Expression<'_>) -> bool {
    match expr {
        Expression::Identifier(_) => true,
        Expression::StaticMemberExpression(sme) => callee_is_safe_receiver(&sme.object),
        _ => false,
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for ApplyCallSimplifier {
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        let Expression::CallExpression(call) = expr else {
            return;
        };
        let Expression::StaticMemberExpression(sme) = &call.callee else {
            return;
        };
        let method = sme.property.name.as_str();
        if method != "apply" && method != "call" {
            return;
        }
        if !callee_is_safe_receiver(&sme.object) {
            return;
        }
        if call.arguments.is_empty() {
            return;
        }
        let Some(first_arg) = call.arguments[0].as_expression() else {
            return;
        };
        if !is_null_or_undefined(first_arg) {
            return;
        }

        let new_callee = sme.object.clone_in_with_semantic_ids(ctx.ast.allocator);
        let new_args: OxcVec<'a, Argument<'a>> = match method {
            "call" => {
                let mut v = ctx.ast.vec();
                for arg in call.arguments.iter().skip(1) {
                    v.push(arg.clone_in_with_semantic_ids(ctx.ast.allocator));
                }
                v
            }
            "apply" => {
                if call.arguments.len() != 2 {
                    return;
                }
                let Some(Expression::ArrayExpression(arr)) = call.arguments[1].as_expression() else {
                    return;
                };
                if arr.elements.iter().any(|e| {
                    matches!(
                        e,
                        ArrayExpressionElement::SpreadElement(_) | ArrayExpressionElement::Elision(_)
                    )
                }) {
                    return;
                }
                let mut v = ctx.ast.vec();
                for el in &arr.elements {
                    if let Some(e) = el.as_expression() {
                        v.push(Argument::from(e.clone_in_with_semantic_ids(ctx.ast.allocator)));
                    } else {
                        return;
                    }
                }
                v
            }
            _ => return,
        };

        if self.rewrites < 10 {
            eprintln!(
                "[AST/apply-call] rewriting .{method}({} args) -> direct call ({} args)",
                call.arguments.len(),
                new_args.len()
            );
        }
        self.rewrites += 1;
        *expr = Expression::CallExpression(ctx.ast.alloc(CallExpression {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            callee: new_callee,
            arguments: new_args,
            optional: false,
            type_arguments: None,
            pure: false,
        }));
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
        let mut pass = ApplyCallSimplifier::new();
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);
        traverse_mut_with_ctx(&mut pass, &mut program, &mut ctx);
        Codegen::new().build(&program).code
    }

    #[test]
    fn apply_with_null_and_array() {
        let out = run("fn.apply(null, [a, b, c]);");
        assert!(!out.contains(".apply"), "got: {out}");
        assert!(out.contains("fn(a, b, c)") || out.contains("fn(a,b,c)"));
    }

    #[test]
    fn apply_with_undefined() {
        let out = run("fn.apply(undefined, [x]);");
        assert!(!out.contains(".apply"), "got: {out}");
    }

    #[test]
    fn call_with_null() {
        let out = run("fn.call(null, a, b);");
        assert!(!out.contains(".call"), "got: {out}");
        assert!(out.contains("fn(a, b)") || out.contains("fn(a,b)"));
    }

    #[test]
    fn leaves_apply_with_non_null_this_alone() {
        let out = run("fn.apply(self, [a]);");
        assert!(out.contains(".apply"), "got: {out}");
    }

    #[test]
    fn leaves_apply_with_non_literal_args_alone() {
        let out = run("fn.apply(null, args);");
        assert!(out.contains(".apply"), "got: {out}");
    }

    #[test]
    fn leaves_spread_in_array_alone() {
        let out = run("fn.apply(null, [a, ...b]);");
        assert!(out.contains(".apply"), "got: {out}");
    }

    #[test]
    fn static_member_receiver() {
        let out = run("obj.method.apply(null, [a, b]);");
        assert!(!out.contains(".apply"), "got: {out}");
        assert!(out.contains("obj.method(a, b)") || out.contains("obj.method(a,b)"));
    }
}
