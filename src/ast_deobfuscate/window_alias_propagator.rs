//! Window-alias propagator: `var Q6 = window; Q6.document` → `document`.
//!
//! Detects variables assigned only to global-like values (`window`, `globalThis`,
//! `global`, `self`, `this`) and replaces `alias.prop` with just `prop`.
//!
//! Aliases are tracked by resolved [`SymbolId`], NOT by raw name. A minified
//! bundle routinely reuses the same short name (`props`, `e`, `t`) for unrelated
//! bindings in different scopes; one `var props = this` must not cause every
//! other function's `props.x` to be stripped. (It also guards against rewriting
//! `alias.in` → `in`, which would emit a reserved word and produce unparseable
//! output.)

use oxc_ast::ast::{
    AssignmentExpression, AssignmentTarget, Expression, ForInStatement, ForOfStatement, IdentifierReference,
    SimpleAssignmentTarget, UpdateExpression, VariableDeclarator,
};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::FxHashSet;
use std::cell::Cell;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

fn is_global_like(expr: &Expression<'_>) -> bool {
    // NOTE: `this` is deliberately NOT global-like. `var self = this; self.foo`
    // inside a method/class reads an instance property — rewriting it to bare
    // `foo` silently turns it into a global lookup (a real miscompile). Only the
    // genuine global objects qualify.
    matches!(expr, Expression::Identifier(id)
        if matches!(id.name.as_str(), "window" | "globalThis" | "global" | "self"))
}

/// A reserved word can never be emitted as a bare identifier, so rewriting
/// `alias.in` → `in` would produce invalid JavaScript. The pipeline always
/// parses as `SourceType::mjs()` (ES module = always strict mode), so the
/// strict-mode future-reserved words (`implements`, `interface`, `package`,
/// `private`, `protected`, `public`) and `async`/`await`/`let`/`static`/`yield`
/// are illegal as bare `IdentifierReference`s too and must be guarded.
fn is_reserved_word(name: &str) -> bool {
    matches!(
        name,
        "break"
            | "case"
            | "catch"
            | "class"
            | "const"
            | "continue"
            | "debugger"
            | "default"
            | "delete"
            | "do"
            | "else"
            | "enum"
            | "export"
            | "extends"
            | "false"
            | "finally"
            | "for"
            | "function"
            | "if"
            | "import"
            | "in"
            | "instanceof"
            | "new"
            | "null"
            | "return"
            | "super"
            | "switch"
            | "this"
            | "throw"
            | "true"
            | "try"
            | "typeof"
            | "var"
            | "void"
            | "while"
            | "with"
            // Strict-mode reserved / contextual keywords (MJS is always strict).
            | "async"
            | "await"
            | "implements"
            | "interface"
            | "let"
            | "package"
            | "private"
            | "protected"
            | "public"
            | "static"
            | "yield"
    )
}

pub struct WindowAliasCollector {
    candidates: FxHashSet<SymbolId>,
    poisoned: FxHashSet<SymbolId>,
}

impl WindowAliasCollector {
    #[must_use]
    pub fn new() -> Self {
        Self {
            candidates: FxHashSet::default(),
            poisoned: FxHashSet::default(),
        }
    }

    #[must_use]
    pub fn aliases(self) -> FxHashSet<SymbolId> {
        &self.candidates - &self.poisoned
    }
}

impl Default for WindowAliasCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for WindowAliasCollector {
    fn enter_variable_declarator(&mut self, decl: &mut VariableDeclarator<'a>, _ctx: &mut Ctx<'a>) {
        let oxc_ast::ast::BindingPattern::BindingIdentifier(id) = &decl.id else {
            return;
        };
        let Some(symbol_id) = id.symbol_id.get() else {
            return;
        };
        // `var X = window` makes X a candidate. A `var X = other()` (or a bare
        // `var X`) poisons it: under `var` hoisting both declarations share ONE
        // SymbolId, so a later non-global redeclaration must cancel the alias.
        // A different binding named X in another scope is a distinct SymbolId.
        match decl.init.as_ref() {
            Some(init) if is_global_like(init) => {
                self.candidates.insert(symbol_id);
            }
            _ => {
                self.poisoned.insert(symbol_id);
            }
        }
    }

    fn enter_assignment_expression(&mut self, assign: &mut AssignmentExpression<'a>, ctx: &mut Ctx<'a>) {
        let AssignmentTarget::AssignmentTargetIdentifier(id) = &assign.left else {
            return;
        };
        let Some(reference_id) = id.reference_id.get() else {
            return;
        };
        let Some(symbol_id) = ctx.scoping().get_reference(reference_id).symbol_id() else {
            return;
        };
        // Only a *simple* `X = window` promotes X. A compound assignment
        // (`X += window` computes `X + window`, a number) or any non-global RHS
        // means X no longer holds a pure global object and must be poisoned.
        if assign.operator.is_assign() && is_global_like(&assign.right) {
            self.candidates.insert(symbol_id);
        } else {
            self.poisoned.insert(symbol_id);
        }
    }

    fn enter_update_expression(&mut self, update: &mut UpdateExpression<'a>, ctx: &mut Ctx<'a>) {
        // `X++` / `--X` rebinds X to a number — it can never stay a global alias.
        let SimpleAssignmentTarget::AssignmentTargetIdentifier(id) = &update.argument else {
            return;
        };
        if let Some(reference_id) = id.reference_id.get()
            && let Some(symbol_id) = ctx.scoping().get_reference(reference_id).symbol_id()
        {
            self.poisoned.insert(symbol_id);
        }
    }

    fn enter_for_in_statement(&mut self, stmt: &mut ForInStatement<'a>, ctx: &mut Ctx<'a>) {
        poison_for_loop_target(&stmt.left, ctx, &mut self.poisoned);
    }

    fn enter_for_of_statement(&mut self, stmt: &mut ForOfStatement<'a>, ctx: &mut Ctx<'a>) {
        poison_for_loop_target(&stmt.left, ctx, &mut self.poisoned);
    }
}

/// `for (X in obj)` / `for (X of it)` rebinds the existing variable `X` on every
/// iteration, so an aliased `X` no longer holds a global object.
fn poison_for_loop_target(
    left: &oxc_ast::ast::ForStatementLeft<'_>,
    ctx: &Ctx<'_>,
    poisoned: &mut FxHashSet<SymbolId>,
) {
    let oxc_ast::ast::ForStatementLeft::AssignmentTargetIdentifier(id) = left else {
        return;
    };
    if let Some(reference_id) = id.reference_id.get()
        && let Some(symbol_id) = ctx.scoping().get_reference(reference_id).symbol_id()
    {
        poisoned.insert(symbol_id);
    }
}

pub struct WindowAliasPropagator {
    aliases: FxHashSet<SymbolId>,
    rewrites: usize,
}

impl WindowAliasPropagator {
    #[must_use]
    pub fn new(aliases: FxHashSet<SymbolId>) -> Self {
        Self { aliases, rewrites: 0 }
    }

    #[must_use]
    pub fn rewrites(&self) -> usize {
        self.rewrites
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for WindowAliasPropagator {
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        let Expression::StaticMemberExpression(sme) = expr else {
            return;
        };
        let Expression::Identifier(obj) = &sme.object else {
            return;
        };
        // Resolve the object to its binding and rewrite ONLY if that exact symbol
        // is a confirmed global alias — a same-named parameter elsewhere resolves
        // to a different SymbolId and is left untouched.
        //
        // Use `reference_id.get()` (not the unwrapping `reference_id()`): a
        // previously-rewritten alias (`alias.a.b` → the inner `alias.a` became a
        // fresh identifier with no reference_id) reaches this node as the object
        // of the outer member expression, and unwrapping its unset id panics.
        let Some(reference_id) = obj.reference_id.get() else {
            return;
        };
        let Some(symbol_id) = ctx.scoping().get_reference(reference_id).symbol_id() else {
            return;
        };
        if !self.aliases.contains(&symbol_id) {
            return;
        }
        let prop_name = sme.property.name.as_str();
        // `alias.in` → `in` would be a bare reserved word: invalid JS. Skip it.
        if is_reserved_word(prop_name) {
            return;
        }
        self.rewrites += 1;
        *expr = Expression::Identifier(ctx.ast.alloc(IdentifierReference {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            name: ctx.ast.ident(prop_name),
            reference_id: Cell::default(),
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

        let mut collector = WindowAliasCollector::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        traverse_mut_with_ctx(&mut collector, &mut program, &mut ctx);

        let aliases = collector.aliases();
        if aliases.is_empty() {
            return Codegen::new().build(&program).code;
        }

        let mut propagator = WindowAliasPropagator::new(aliases);
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        traverse_mut_with_ctx(&mut propagator, &mut program, &mut ctx);

        Codegen::new().build(&program).code
    }

    #[test]
    fn propagates_window_alias() {
        let out = run("var Q6 = window; var x = Q6.document;");
        assert!(
            out.contains("var x = document"),
            "expected Q6.document → document, got: {out}"
        );
        assert!(!out.contains("Q6.document"), "Q6.document should be gone, got: {out}");
    }

    #[test]
    fn preserves_reassigned_alias() {
        let out = run("var Q6 = window; Q6 = other; var x = Q6.document;");
        assert!(
            out.contains("Q6.document"),
            "reassigned alias should not be propagated, got: {out}"
        );
    }

    #[test]
    fn poisons_compound_assignment_with_global_rhs() {
        // `w += window` computes `w + window` (a number/NaN) — w is NOT a global
        // alias, so `w.document` must stay qualified.
        let out = run("var w = window; w += window; var y = w.document;");
        assert!(
            out.contains("w.document"),
            "compound-assigned var must not propagate: {out}"
        );
    }

    #[test]
    fn poisons_var_redeclaration_to_non_global() {
        // `var` hoisting: both decls share one SymbolId; the later non-global
        // redeclaration cancels the alias.
        let out = run("var X = window; var X = other(); var y = X.document;");
        assert!(out.contains("X.document"), "redeclared var must not propagate: {out}");
    }

    #[test]
    fn poisons_update_expression() {
        let out = run("var X = window; X++; var y = X.document;");
        assert!(out.contains("X.document"), "incremented var must not propagate: {out}");
    }

    #[test]
    fn poisons_for_in_loop_target() {
        let out = run("var X = window; for (X in obj) {} var y = X.document;");
        assert!(out.contains("X.document"), "for-in loop var must not propagate: {out}");
    }

    #[test]
    fn poisons_for_of_loop_target() {
        let out = run("var X = window; for (X of arr) {} var y = X.document;");
        assert!(out.contains("X.document"), "for-of loop var must not propagate: {out}");
    }

    #[test]
    fn preserves_computed_access() {
        let out = run("var Q6 = window; var x = Q6[\"doc\"];");
        assert!(
            out.contains("Q6[") || out.contains("Q6."),
            "computed access should not be rewritten, got: {out}"
        );
    }

    #[test]
    fn does_not_clobber_same_named_binding_in_other_scope() {
        // A `var props = window` in one function must NOT cause an unrelated
        // function's `props` PARAMETER to have its `props.in` stripped to the
        // reserved word `in` (which would crash the re-parse checkpoint). This is
        // the core SymbolId-scoping guarantee.
        let code = "function a(){ var props = window; return props.foo; } \
                    function b(props){ if (props.in) { return props.enter; } }";
        let out = run(code);
        assert!(out.contains("props.in"), "param props.in must survive: {out}");
        assert!(out.contains("props.enter"), "param props.enter must survive: {out}");
        assert!(!out.contains("if (in)"), "must never emit bare reserved word: {out}");
    }

    #[test]
    fn does_not_alias_this_as_global() {
        // `var self = this; self.value` reads an instance property — it must NOT
        // become a bare global `value`. `this` is not a global object.
        let out = run("class C { m(){ var self = this; return self.value; } }");
        assert!(
            out.contains("self.value"),
            "this-alias receiver must be preserved: {out}"
        );
        assert!(!out.contains("return value"), "must not strip the this receiver: {out}");
    }

    #[test]
    fn handles_nested_member_access_on_alias_without_panicking() {
        // `alias.a.b`: the inner `alias.a` is rewritten to a bare `a` whose
        // reference_id is unset; visiting the outer member expression must not
        // unwrap that missing id (regression for a get_id.rs unwrap panic).
        let out = run("var Q6 = window; var x = Q6.location.href;");
        assert!(out.contains("location.href"), "nested access rewritten: {out}");
        assert!(!out.contains("Q6.location"), "alias object should be gone: {out}");
    }

    #[test]
    fn does_not_emit_reserved_word_even_for_real_alias() {
        // Even when `props` IS the alias, `props.in` cannot become bare `in`.
        let out = run("var props = window; if (props.in) { x(); }");
        assert!(out.contains("props.in"), "reserved property must stay qualified: {out}");
        assert!(!out.contains("if (in)"), "must not emit bare reserved word: {out}");
    }

    #[test]
    fn does_not_emit_strict_mode_reserved_words() {
        // MJS is always strict, so `async`/`implements`/etc. are illegal as bare
        // identifier references even though they're valid property names.
        for kw in [
            "async",
            "implements",
            "interface",
            "package",
            "private",
            "protected",
            "public",
            "static",
            "let",
            "await",
            "yield",
            "in",
            "for",
            "class",
            "return",
        ] {
            let out = run(&format!("var w = window; var r = w.{kw};"));
            assert!(
                out.contains(&format!("w.{kw}")),
                "{kw}: reserved word must stay qualified, got: {out}"
            );
        }
    }
}
