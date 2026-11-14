//! Bindings that a `with` statement may hide behind a property of its object.
//!
//! Inside `with (obj) { … }` a name first resolves to `obj`'s property of that
//! name, when there is one, and only otherwise to the binding `oxc_semantic`
//! resolves it to. A pass that renames such a binding, or replaces such a
//! read with the binding's value, changes which of the two the program reads.

use std::cell::Cell;

use oxc_ast::ast::{Program, VariableDeclarator};
use oxc_ast_visit::{Visit, walk};
use oxc_semantic::Scoping;
use oxc_syntax::scope::{ScopeFlags, ScopeId};
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::TraverseCtx;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::annex_b::annex_b_shadowed_symbols;

/// Bindings whose references may reach something other than their own
/// declaration at runtime: a sloppy-mode block function
/// ([`annex_b_shadowed_symbols`]) or a `with` object's property
/// ([`with_observed_symbols`]). A pass must not replace their references with
/// what the declaration holds.
#[must_use]
pub fn redirectable_symbols(program: &Program<'_>, scoping: &Scoping) -> FxHashSet<SymbolId> {
    let mut symbols = annex_b_shadowed_symbols(program, scoping);
    symbols.extend(with_observed_symbols(program, scoping));
    symbols
}

/// Bindings a `with` body reads or writes by name past its `with` object:
/// the bindings of references inside the body, and the `var`s the body
/// declares (`var x = 1` there assigns `obj.x` when `obj` has an `x`). A
/// binding declared inside the body, such as a block's `let` or a nested
/// function's parameter, is nearer than the object and is not returned.
///
/// Unresolved references need nothing here: they read the object's property
/// or a global, and passes never bind a name a free reference reads.
#[must_use]
pub fn with_observed_symbols(program: &Program<'_>, scoping: &Scoping) -> FxHashSet<SymbolId> {
    if !has_with(scoping) {
        return FxHashSet::default();
    }
    let nesting = WithNesting::new(scoping);
    let mut declarations = VarDeclarations {
        scopes: Vec::new(),
        found: Vec::new(),
    };
    declarations.visit_program(program);
    let declared = declarations.found.into_iter();
    let referenced = scoping.symbol_ids().flat_map(|symbol| {
        scoping
            .get_resolved_reference_ids(symbol)
            .iter()
            .map(move |&reference| (scoping.get_reference(reference).scope_id(), symbol))
    });
    declared
        .chain(referenced)
        .filter(|&(scope, symbol)| nesting.passes_with(scope, scoping.symbol_scope_id(symbol)))
        .map(|(_, symbol)| symbol)
        .collect()
}

/// Scopes inside a `with` body, where a name may read a property of the
/// `with` object. A pass that matches bindings by name, not by `SymbolId`,
/// must leave a reference in one of these scopes alone.
#[must_use]
pub fn scopes_in_with_bodies(scoping: &Scoping) -> FxHashSet<ScopeId> {
    if !has_with(scoping) {
        return FxHashSet::default();
    }
    WithNesting::new(scoping)
        .0
        .into_iter()
        .filter_map(|(scope, (_, with))| with.map(|_| scope))
        .collect()
}

/// Whether a bare `name` at the current position of a traversal reads the
/// global object's property: no binding of that name is visible there and no
/// `with` body encloses it. A pass that writes `undefined`, `NaN` or
/// `Infinity`, or reads a builtin such as `String`, by name needs this.
#[must_use]
pub fn is_global_reference<State>(ctx: &TraverseCtx<'_, State>, name: &str) -> bool {
    let scoping = ctx.scoping();
    ctx.ancestor_scopes()
        .all(|scope| !scoping.scope_flags(scope).is_with() && scoping.get_binding(scope, name.into()).is_none())
}

fn has_with(scoping: &Scoping) -> bool {
    scoping
        .scope_descendants_from_root()
        .any(|scope| scoping.scope_flags(scope).is_with())
}

/// Per scope, its depth and its nearest enclosing (or own) `with` body scope.
struct WithNesting(FxHashMap<ScopeId, (usize, Option<ScopeId>)>);

impl WithNesting {
    fn new(scoping: &Scoping) -> Self {
        let mut nesting = FxHashMap::default();
        for scope in scoping.scope_descendants_from_root() {
            let mut chain = Vec::new();
            let mut current = Some(scope);
            while let Some(id) = current
                && !nesting.contains_key(&id)
            {
                chain.push(id);
                current = scoping.scope_parent_id(id);
            }
            let (mut depth, mut with) = current.and_then(|id| nesting.get(&id).copied()).unwrap_or((0, None));
            for id in chain.into_iter().rev() {
                depth += 1;
                if scoping.scope_flags(id).is_with() {
                    with = Some(id);
                }
                nesting.insert(id, (depth, with));
            }
        }
        Self(nesting)
    }

    /// Whether a name used in `scope` and bound in its ancestor `binding`
    /// is looked up in a `with` object first.
    fn passes_with(&self, scope: ScopeId, binding: ScopeId) -> bool {
        let depth = |scope| self.0.get(&scope).map(|&(depth, _)| depth);
        self.0
            .get(&scope)
            .and_then(|&(_, with)| with)
            .and_then(depth)
            .is_some_and(|with_depth| depth(binding).is_none_or(|binding_depth| with_depth >= binding_depth))
    }
}

/// Every variable declarator binding with the scope it is declared in.
struct VarDeclarations {
    scopes: Vec<ScopeId>,
    found: Vec<(ScopeId, SymbolId)>,
}

impl<'a> Visit<'a> for VarDeclarations {
    fn enter_scope(&mut self, _flags: ScopeFlags, scope_id: &Cell<Option<ScopeId>>) {
        if let Some(scope) = scope_id.get() {
            self.scopes.push(scope);
        }
    }

    fn leave_scope(&mut self) {
        self.scopes.pop();
    }

    fn visit_variable_declarator(&mut self, declarator: &VariableDeclarator<'a>) {
        if let Some(&scope) = self.scopes.last() {
            self.found.extend(
                declarator
                    .id
                    .get_binding_identifiers()
                    .into_iter()
                    .filter_map(|id| id.symbol_id.get())
                    .map(|symbol| (scope, symbol)),
            );
        }
        walk::walk_variable_declarator(self, declarator);
    }
}

#[cfg(test)]
mod tests {
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    use super::with_observed_symbols;
    use crate::ast_deobfuscate::semantic;

    /// The observed bindings' names, in declaration order.
    fn names(code: &str) -> Vec<String> {
        let allocator = Allocator::default();
        let program = Parser::new(&allocator, code, SourceType::cjs()).parse().program;
        let scoping = semantic::scoping(&program);
        let mut found: Vec<_> = with_observed_symbols(&program, &scoping).into_iter().collect();
        found.sort_unstable();
        found
            .into_iter()
            .map(|symbol| scoping.symbol_name(symbol).to_string())
            .collect()
    }

    #[test]
    fn guards_bindings_read_from_a_with_body() {
        let code = "function g(o, p) { function D() {} var x, y; with (o) { D(); (function () { return x; })(); } return y + p; }";
        assert_eq!(names(code), ["D", "x"]);
    }

    #[test]
    fn guards_vars_a_with_body_declares() {
        assert_eq!(
            names("function g(o) { with (o) { var k = 1; for (var i in []) {} } }"),
            ["k", "i"]
        );
    }

    #[test]
    fn leaves_bindings_inside_the_body_and_the_object_alone() {
        for code in [
            "function g(o) { with (o) { let a = 1; a; } }",
            "function g(o) { with (o) (function (p) { var q; return p + q; })(); }",
            "function g(o) { with (o) { try {} catch (e) { e; } } }",
            "function g() { var o = {}; with (o) {} }",
            "function g(o) { var x; return x; }",
        ] {
            assert!(names(code).is_empty(), "{code}: {:?}", names(code));
        }
    }

    #[test]
    fn nested_with_bodies_guard_through_the_inner_one() {
        // `p`, the inner object, is itself read in the outer body.
        let code = "function g(o, p) { var x; with (o) { (function () { var y; with (p) { x; y; } })(); } }";
        assert_eq!(names(code), ["p", "x", "y"]);
    }
}
