//! `.call(this, ...)` → direct call simplifier for plain identifiers.
//!
//! Rewrites `X.call(this, ARG1, ARG2, ...)` → `X(ARG1, ARG2, ...)` when `X`
//! is a never-reassigned binding of an arrow function, or of a function whose
//! own `this` is never observed. Any other callee would see a different `this`.

use oxc_allocator::{CloneIn, GetAllocator, Vec as OxcVec};
use oxc_ast::ast::{Argument, Expression, Program};
use oxc_span::SPAN;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::FxHashSet;

use crate::ast_deobfuscate::function_context::this_free_functions;
use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct CallThisSimplifier {
    rewrites: usize,
    this_free: FxHashSet<SymbolId>,
}

impl CallThisSimplifier {
    #[must_use]
    pub fn new() -> Self {
        Self {
            rewrites: 0,
            this_free: FxHashSet::default(),
        }
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
    fn enter_program(&mut self, program: &mut Program<'a>, ctx: &mut Ctx<'a>) {
        self.this_free = this_free_functions(program, ctx.scoping());
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
        let Some(symbol) = ident
            .reference_id
            .get()
            .and_then(|reference| ctx.scoping().get_reference(reference).symbol_id())
        else {
            return;
        };
        if !self.this_free.contains(&symbol) {
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
        run_with(code, SourceType::mjs())
    }

    fn run_with(code: &str, source_type: SourceType) -> String {
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, code, source_type).parse();
        let mut program = ret.program;
        let mut pass = CallThisSimplifier::new();
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);
        traverse_mut_with_ctx(&mut pass, &mut program, &mut ctx);
        Codegen::new().build(&program).code
    }

    fn assert_unchanged(code: &str) {
        let out = run(code);
        assert!(
            out.contains(".call(this"),
            "expected .call(this, ...) preserved, got: {out}"
        );
    }

    #[test]
    fn this_free_function_call_simplified() {
        let out = run("function P6(a, b) { return a + b; }\nP6.call(this, a, b);");
        assert!(out.contains("P6(a, b);"), "expected P6(a, b), got: {out}");
    }

    #[test]
    fn this_inside_nested_function_does_not_count() {
        let out = run("function f(a) { return function () { return this; }; }\nf.call(this, 1);");
        assert!(out.contains("f(1);"), "got: {out}");
    }

    #[test]
    fn arrow_ignores_this_argument() {
        let out = run("const f = () => this;\nf.call(this, 1);");
        assert!(out.contains("f(1);"), "got: {out}");
    }

    #[test]
    fn function_reading_this_preserved() {
        assert_unchanged(
            "function fl() { return { ...this.opts }; }\nfunction Ajv(o) { this.opts = o; this._metaOpts = fl.call(this); }",
        );
        assert_unchanged("var f = function () { return this.x; };\nf.call(this);");
        assert_unchanged("function f() { return () => this.x; }\nf.call(this);");
        assert_unchanged("function f(a = this) { return a; }\nf.call(this);");
    }

    #[test]
    fn direct_eval_preserved() {
        assert_unchanged("function f(s) { return eval(s); }\nf.call(this, \"this\");");
    }

    #[test]
    fn unknown_or_rebound_callee_preserved() {
        assert_unchanged("P6.call(this, a, b);");
        assert_unchanged("function f() { return 1; }\nf = g;\nf.call(this);");
        assert_unchanged("function f() { return 1; }\nfunction h(f) { return f.call(this); }");
        assert_unchanged("var f = () => 1;\nvar f = function () { return this; };\nf.call(this);");
    }

    #[test]
    fn script_global_function_preserved() {
        let code = "function P6(a) { return a; }\nP6.call(this, a);";
        let out = run_with(code, SourceType::script());
        assert!(out.contains(".call(this"), "another script may rebind P6: {out}");
        let out = run_with(&format!("(function () {{ {code} }})();"), SourceType::script());
        assert!(out.contains("P6(a)"), "a function-local P6 is simplified: {out}");
    }

    #[test]
    fn member_call_this_preserved() {
        let out = run("obj.method.call(this, a);");
        assert!(out.contains(".call"), "expected .call preserved, got: {out}");
    }

    #[test]
    fn call_null_not_affected() {
        let out = run("function f(a) { return a; }\nf.call(null, a);");
        assert!(
            out.contains(".call"),
            "expected .call preserved for null receiver, got: {out}"
        );
    }

    #[test]
    fn spread_arg_preserved() {
        let out = run("function f(a) { return a; }\nf.call(this, ...args);");
        assert!(out.contains(".call"), "expected .call preserved for spread, got: {out}");
    }
}
