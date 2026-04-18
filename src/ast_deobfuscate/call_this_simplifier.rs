//! `.call(this, ...)` → direct call simplifier for plain identifiers.
//!
//! Rewrites `X.call(this, ARG1, ARG2, ...)` → `X(ARG1, ARG2, ...)`
//! when `X` is a plain identifier (not a member expression).
//!
//! Targets the Akamai BMP pattern where trampoline-inlined call sites produce
//! `P6.call(this, NA, [U8.length, ZW])` — the `.call(this, ...)` is redundant
//! because `P6` is a top-level function that doesn't use `this`.

use oxc_allocator::CloneIn;
use oxc_ast::ast::{Argument, CallExpression, Expression};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use oxc_traverse::{Traverse, TraverseCtx};
use std::cell::Cell;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct CallThisSimplifier {
    rewrites: usize,
}

impl CallThisSimplifier {
    #[must_use]
    pub const fn new() -> Self {
        Self { rewrites: 0 }
    }

    #[must_use]
    pub const fn rewrites(&self) -> usize {
        self.rewrites
    }
}

impl Default for CallThisSimplifier {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for CallThisSimplifier {
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        let Expression::CallExpression(call) = expr else {
            return;
        };
        let Expression::StaticMemberExpression(sme) = &call.callee else {
            return;
        };
        if sme.property.name.as_str() != "call" {
            return;
        }
        let Expression::Identifier(ref ident) = sme.object else {
            return;
        };
        if call.arguments.is_empty() {
            return;
        }
        let Some(Expression::ThisExpression(_)) = call.arguments[0].as_expression() else {
            return;
        };
        if call
            .arguments
            .iter()
            .skip(1)
            .any(|a| matches!(a, Argument::SpreadElement(_)))
        {
            return;
        }

        let callee_name = ident.name.as_str().to_string();
        let new_callee = sme.object.clone_in_with_semantic_ids(ctx.ast.allocator);
        let mut new_args = ctx.ast.vec();
        for arg in call.arguments.iter().skip(1) {
            new_args.push(arg.clone_in_with_semantic_ids(ctx.ast.allocator));
        }

        if self.rewrites < 10 {
            eprintln!("[AST/call-this] rewriting {callee_name}.call(this, ...) -> {callee_name}(...)");
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
        let mut pass = CallThisSimplifier::new();
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);
        traverse_mut_with_ctx(&mut pass, &mut program, &mut ctx);
        Codegen::new().build(&program).code
    }

    #[test]
    fn identifier_call_this_simplified() {
        let out = run("P6.call(this, a, b);");
        assert!(!out.contains(".call"), "expected no .call, got: {out}");
        assert!(
            out.contains("P6(a, b)") || out.contains("P6(a,b)"),
            "expected P6(a, b), got: {out}"
        );
    }

    #[test]
    fn member_call_this_preserved() {
        let out = run("obj.method.call(this, a);");
        assert!(out.contains(".call"), "expected .call preserved, got: {out}");
    }

    #[test]
    fn call_null_not_affected() {
        let out = run("f.call(null, a);");
        assert!(
            out.contains(".call"),
            "expected .call preserved for null receiver, got: {out}"
        );
    }

    #[test]
    fn spread_arg_preserved() {
        let out = run("f.call(this, ...args);");
        assert!(out.contains(".call"), "expected .call preserved for spread, got: {out}");
    }
}
