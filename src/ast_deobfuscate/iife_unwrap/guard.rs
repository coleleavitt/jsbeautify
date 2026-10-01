//! Soundness guard for splicing an arrow IIFE body into its enclosing scope.
//!
//! Unwrapping `(() => { var a; let b; function c() {} })();` moves `a`, `b`
//! and `c` out of the arrow's private scope into the statement list that
//! contained the call. That is only behavior-preserving when none of those
//! names is observable there:
//!
//! - another statement in the same list (including a sibling IIFE that is also
//!   being unwrapped) binds or references the name — a redeclaration error, or
//!   a silent `var`/`var` merge that clobbers state or captures a reference;
//! - an ancestor scope binds the name (parameters included), which the hoisted
//!   binding would shadow or redeclare;
//! - the list is a script's top level, where `var`/`function` become global
//!   object properties and `let`/`const` join the shared global lexical record;
//! - the list is a block, where `var` and (Annex B) function declarations would
//!   escape into the enclosing function and be shared across loop iterations,
//!   and inner labels could duplicate an enclosing label.
//!
//! Direct `eval` and directive prologues are rejected outright: eval resolves
//! names dynamically, and a `"use strict"` prologue would be dropped.

use oxc_ast::ast::{
    ArrowFunctionExpression, BindingIdentifier, Class, Function, FunctionBody, Ident, IdentifierReference,
    LabeledStatement, Statement, VariableDeclaration,
};
use oxc_ast_visit::{Visit, walk};
use oxc_syntax::scope::{ScopeFlags, ScopeId};
use rustc_hash::FxHashMap;

use super::Ctx;

/// Kind of statement list the arrow body is spliced into.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Target {
    /// Top level of a script (sloppy global scope).
    ScriptTop,
    /// Top level of a module, or a function body.
    VarScope,
    /// A nested block statement.
    Block,
}

/// Every identifier name (bindings and references) in a statement list, with
/// occurrence counts so one statement's own names can be subtracted out.
#[derive(Default)]
pub(super) struct NameCounts(FxHashMap<String, u32>);

impl NameCounts {
    pub(super) fn of_statements(stmts: &[Statement<'_>]) -> Self {
        let mut counts = Self::default();
        for stmt in stmts {
            counts.visit_statement(stmt);
        }
        counts
    }

    fn bump(&mut self, name: &str) {
        let slot = self.0.entry(name.to_string()).or_insert(0);
        *slot = slot.saturating_add(1);
    }

    fn get(&self, name: &str) -> u32 {
        self.0.get(name).copied().unwrap_or(0)
    }
}

impl<'a> Visit<'a> for NameCounts {
    fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
        self.bump(it.name.as_str());
    }

    fn visit_binding_identifier(&mut self, it: &BindingIdentifier<'a>) {
        self.bump(it.name.as_str());
    }
}

/// Names an arrow body would introduce into the enclosing scope once spliced.
#[derive(Default)]
struct HoistedNames {
    names: Vec<String>,
    /// A `var` or function declaration (var-scoped under Annex B) exists.
    var_scoped: bool,
    has_label: bool,
}

impl HoistedNames {
    fn collect(body: &FunctionBody<'_>) -> Self {
        let mut hoisted = Self::default();
        for stmt in &body.statements {
            match stmt {
                // Top-level lexical declarations land in the target scope too.
                Statement::VariableDeclaration(decl) => hoisted.push_declaration(decl),
                Statement::ClassDeclaration(class) => {
                    if let Some(id) = &class.id {
                        hoisted.names.push(id.name.to_string());
                    }
                }
                _ => hoisted.visit_statement(stmt),
            }
        }
        hoisted
    }

    fn push_declaration(&mut self, decl: &VariableDeclaration<'_>) {
        for declarator in &decl.declarations {
            for binding in declarator.id.get_binding_identifiers() {
                self.names.push(binding.name.to_string());
            }
        }
        self.var_scoped |= decl.kind.is_var();
    }
}

impl<'a> Visit<'a> for HoistedNames {
    // Nested `var` (including `for (var …)` heads) escapes its block; nested
    // `let`/`const` stay block scoped and are not hoisted.
    fn visit_variable_declaration(&mut self, it: &VariableDeclaration<'a>) {
        if it.kind.is_var() {
            self.push_declaration(it);
        }
    }

    fn visit_function(&mut self, it: &Function<'a>, _flags: ScopeFlags) {
        if it.is_declaration() {
            if let Some(id) = &it.id {
                self.names.push(id.name.to_string());
            }
            self.var_scoped = true;
        }
    }

    fn visit_arrow_function_expression(&mut self, _it: &ArrowFunctionExpression<'a>) {}

    // Nested class declarations are block scoped; class bodies cannot contain
    // `var` outside static blocks, which have their own var scope.
    fn visit_class(&mut self, _it: &Class<'a>) {}

    fn visit_labeled_statement(&mut self, it: &LabeledStatement<'a>) {
        self.has_label = true;
        walk::walk_labeled_statement(self, it);
    }
}

/// Whether `body` (the arrow IIFE inside `candidate`) can be spliced into a
/// statement list of kind `target` whose scope is `target_scope`.
///
/// `all_counts` covers the whole original statement list, `candidate` included.
pub(super) fn can_splice<'a>(
    body: &FunctionBody<'a>,
    candidate: &Statement<'a>,
    all_counts: &NameCounts,
    target: Target,
    target_scope: ScopeId,
    ctx: &Ctx<'a>,
) -> bool {
    if !body.directives.is_empty() {
        return false;
    }
    let own_counts = NameCounts::of_statements(std::slice::from_ref(candidate));
    if own_counts.get("eval") > 0 {
        return false;
    }

    let hoisted = HoistedNames::collect(body);
    if target == Target::Block && (hoisted.var_scoped || hoisted.has_label) {
        return false;
    }
    if hoisted.names.is_empty() {
        return true;
    }
    if target == Target::ScriptTop {
        return false;
    }

    hoisted.names.iter().all(|name| {
        let name = name.as_str();
        // Each hoisted name must occur only inside the IIFE itself: not in a
        // sibling statement, and not in the assigned declarator's own pattern.
        let outside_iife =
            all_counts.get(name).saturating_sub(own_counts.get(name)) + declarator_pattern_count(candidate, name);
        name != "arguments"
            && outside_iife == 0
            && ctx
                .scoping()
                .find_binding(target_scope, Ident::from_str_in(name, ctx))
                .is_none()
    })
}

/// Occurrences of `name` in the binding pattern of `const x = (() => …)()`;
/// the pattern stays outside the spliced body, so a hoisted name there would
/// be captured (e.g. `const { a = q } = (() => { let q; … })()`).
fn declarator_pattern_count(candidate: &Statement<'_>, name: &str) -> u32 {
    let Statement::VariableDeclaration(decl) = candidate else {
        return 0;
    };
    let mut counts = NameCounts::default();
    for declarator in &decl.declarations {
        counts.visit_binding_pattern(&declarator.id);
    }
    counts.get(name)
}
