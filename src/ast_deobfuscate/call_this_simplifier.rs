//! `.call(this, ...)` → direct call simplifier for plain identifiers.
//!
//! Rewrites `X.call(this, ARG1, ARG2, ...)` → `X(ARG1, ARG2, ...)` when `X`
//! resolves to a never-reassigned function declaration whose body cannot
//! observe its receiver: it contains no `this` (nested functions included) and
//! no direct `eval`.
//!
//! Targets the Akamai BMP pattern where trampoline-inlined call sites produce
//! `P6.call(this, NA, [U8.length, ZW])` — the `.call(this, ...)` is redundant
//! because `P6` is a top-level function that doesn't use `this`.

use oxc_allocator::{CloneIn, GetAllocator, Vec as OxcVec};
use oxc_ast::ast::{Argument, CallExpression, Expression, Function, Program, ThisExpression};
use oxc_ast_visit::{Visit, walk};
use oxc_span::SPAN;
use oxc_syntax::scope::ScopeFlags;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::FxHashSet;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

#[derive(Default)]
pub struct CallThisSimplifier {
    rewrites: usize,
    receiver_free: FxHashSet<SymbolId>,
}

impl CallThisSimplifier {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub const fn rewrites(&self) -> usize {
        self.rewrites
    }
}

/// Function declarations that never observe their receiver. A declaration is
/// tainted when `this` or a direct `eval` appears anywhere inside it; a symbol
/// declared more than once qualifies only if no declaration is tainted.
#[derive(Default)]
struct ReceiverFreeFunctions {
    open: Vec<(Option<SymbolId>, bool)>,
    clean: FxHashSet<SymbolId>,
    tainted: FxHashSet<SymbolId>,
}

impl ReceiverFreeFunctions {
    fn collect(program: &Program<'_>) -> FxHashSet<SymbolId> {
        let mut collector = Self::default();
        collector.visit_program(program);
        let Self { mut clean, tainted, .. } = collector;
        clean.retain(|symbol_id| !tainted.contains(symbol_id));
        clean
    }

    fn taint_open(&mut self) {
        for (_, tainted) in &mut self.open {
            *tainted = true;
        }
    }
}

impl<'a> Visit<'a> for ReceiverFreeFunctions {
    fn visit_function(&mut self, func: &Function<'a>, flags: ScopeFlags) {
        if !func.is_declaration() {
            walk::walk_function(self, func, flags);
            return;
        }
        self.open
            .push((func.id.as_ref().and_then(|id| id.symbol_id.get()), false));
        walk::walk_function(self, func, flags);
        let Some((symbol_id, tainted)) = self.open.pop() else {
            return;
        };
        if let Some(symbol_id) = symbol_id {
            if tainted {
                self.tainted.insert(symbol_id);
            } else {
                self.clean.insert(symbol_id);
            }
        }
    }

    fn visit_this_expression(&mut self, _it: &ThisExpression) {
        self.taint_open();
    }

    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        if let Expression::Identifier(id) = call.callee.without_parentheses()
            && id.name == "eval"
        {
            self.taint_open();
        }
        walk::walk_call_expression(self, call);
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for CallThisSimplifier {
    fn enter_program(&mut self, program: &mut Program<'a>, _ctx: &mut Ctx<'a>) {
        self.receiver_free = ReceiverFreeFunctions::collect(program);
    }

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
        let Some(symbol_id) = ctx.scoping().get_reference(ident.reference_id()).symbol_id() else {
            return;
        };
        if !self.receiver_free.contains(&symbol_id) || ctx.scoping().symbol_is_mutated(symbol_id) {
            return;
        }
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
        let new_callee = sme.object.clone_in_with_semantic_ids(ctx.allocator());
        let mut new_args = OxcVec::new_in(ctx);
        for arg in call.arguments.iter().skip(1) {
            new_args.push(arg.clone_in_with_semantic_ids(ctx.allocator()));
        }

        if self.rewrites < 10 {
            eprintln!("[AST/call-this] rewriting {callee_name}.call(this, ...) -> {callee_name}(...)");
        }
        self.rewrites += 1;
        *expr = Expression::new_call_expression(SPAN, new_callee, None, new_args, false, ctx);
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
        let out = run("function P6(s, a) { return s + a.length; } P6.call(this, a, b);");
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
        let out = run("function f(a) { return a; } f.call(null, a);");
        assert!(
            out.contains(".call"),
            "expected .call preserved for null receiver, got: {out}"
        );
    }

    #[test]
    fn spread_arg_preserved() {
        let out = run("function f(a) { return a; } f.call(this, ...args);");
        assert!(out.contains(".call"), "expected .call preserved for spread, got: {out}");
    }

    #[test]
    fn callee_using_this_preserved() {
        for code in [
            "function f(n) { return this.k + n; } f.call(this, 1);",
            "function f(n) { return () => this.k + n; } f.call(this, 1);",
            "function f(n) { return eval(\"this\"); } f.call(this, 1);",
        ] {
            let out = run(code);
            assert!(out.contains(".call(this"), "{code} -> {out}");
        }
    }

    #[test]
    fn non_declaration_callee_preserved() {
        for code in [
            "function g(t, n) { return t.call(this, n); }",
            "const f = function(n) { return n; }; f.call(this, 1);",
            "f.call(this, 1);",
        ] {
            let out = run(code);
            assert!(out.contains(".call(this"), "{code} -> {out}");
        }
    }

    #[test]
    fn reassigned_callee_preserved() {
        let out = run("function f(n) { return n; } f = function(n) { return this.k + n; }; f.call(this, 1);");
        assert!(out.contains(".call(this"), "got: {out}");
    }

    #[test]
    fn shadowing_callee_preserved() {
        let out = run("function f(n) { return n; } function g(f) { return f.call(this, 1); }");
        assert!(out.contains(".call(this"), "got: {out}");
    }
}
