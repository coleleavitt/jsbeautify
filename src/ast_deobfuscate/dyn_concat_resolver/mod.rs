//! Dynamic-concatenation resolver: inlines string-valued bindings into
//! `+` chains and template-literal interpolations so endpoint extractors see
//! the full URL pattern.
//!
//! A binding is tracked when its declarator is `NAME = <string>` and the
//! symbol is never written again or redeclared. A reference is replaced only
//! when its value is certain to be initialised by the time the reference runs:
//!  - the declarator was visited earlier in this traversal (textually before),
//!  - it sits directly in its binding scope and not in a `switch` case, so
//!    reaching any later point of that scope means it has executed,
//!  - the reference is not inside a function declaration hoisted into that
//!    scope, which could run before the declarator.
//!
//! Top-level `var`/`let` in scripts can be reassigned through the global
//! object or another script, so only module top levels admit them. Programs
//! with a direct `eval` or a `with` statement are left untouched.
//!
//! A `+` chain is only treated as string concatenation if at least one operand
//! is a string literal or a tracked binding.

use oxc_ast::ast::{BinaryOperator, Expression, Function, Program, VariableDeclarator};
use oxc_syntax::scope::{ScopeFlags, ScopeId};
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{Ancestor, Traverse, TraverseCtx};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub(super) const MAX_INLINE_LEN: usize = 256;

pub mod literal;
mod resolve;

use resolve::{
    chain_has_string_evidence, fold_adjacent_literals, substitute_in_chain, substitute_in_template,
    try_resolve_to_string,
};

#[derive(Default)]
pub struct DynConcatResolver {
    enabled: bool,
    module: bool,
    idents: FxHashMap<SymbolId, String>,
    function_declaration_scopes: FxHashSet<ScopeId>,
    rewrites: usize,
    /// Also set by folding adjacent literals, which `rewrites` does not count.
    changed: bool,
}

impl DynConcatResolver {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    #[must_use]
    pub const fn rewrites(&self) -> usize {
        self.rewrites
    }

    #[must_use]
    pub const fn changed(&self) -> bool {
        self.changed
    }

    fn lookup_ident(&self, symbol_id: SymbolId, ctx: &Ctx<'_>) -> Option<&str> {
        let value = self.idents.get(&symbol_id)?;
        let scoping = ctx.scoping();
        let binding_scope = scoping.symbol_scope_id(symbol_id);
        for scope in scoping.scope_ancestors(ctx.current_scope_id()) {
            if scope == binding_scope {
                return Some(value);
            }
            if self.function_declaration_scopes.contains(&scope)
                && scoping.scope_parent_id(scope) == Some(binding_scope)
            {
                return None;
            }
        }
        None
    }

    fn admits_declarator(&self, symbol_id: SymbolId, ctx: &Ctx<'_>) -> bool {
        let scoping = ctx.scoping();
        let binding_scope = scoping.symbol_scope_id(symbol_id);
        !scoping.symbol_is_mutated(symbol_id)
            && scoping.symbol_redeclarations(symbol_id).is_empty()
            && ctx.current_scope_id() == binding_scope
            && !matches!(ctx.ancestor(1), Ancestor::SwitchCaseConsequent(_))
            && (scoping.symbol_flags(symbol_id).is_const_variable()
                || self.module
                || binding_scope != scoping.root_scope_id())
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for DynConcatResolver {
    fn enter_program(&mut self, program: &mut Program<'a>, ctx: &mut Ctx<'a>) {
        let scoping = ctx.scoping();
        self.enabled = !scoping.scope_descendants_from_root().any(|scope| {
            scoping
                .scope_flags(scope)
                .intersects(ScopeFlags::DirectEval | ScopeFlags::With)
        });
        self.module = program.source_type.is_module();
        self.idents.clear();
        self.function_declaration_scopes.clear();
    }

    fn enter_function(&mut self, func: &mut Function<'a>, _ctx: &mut Ctx<'a>) {
        if func.is_declaration() {
            self.function_declaration_scopes.insert(func.scope_id());
        }
    }

    fn exit_variable_declarator(&mut self, decl: &mut VariableDeclarator<'a>, ctx: &mut Ctx<'a>) {
        if !self.enabled {
            return;
        }
        let Some(ident) = decl.id.get_binding_identifier() else {
            return;
        };
        let symbol_id = ident.symbol_id();
        let Some(init) = &decl.init else {
            return;
        };
        if !self.admits_declarator(symbol_id, ctx) {
            return;
        }
        if let Some(value) = try_resolve_to_string(init, self, ctx)
            && value.len() <= MAX_INLINE_LEN
        {
            self.idents.insert(symbol_id, value);
        }
    }

    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        if !self.enabled {
            return;
        }
        match expr {
            Expression::BinaryExpression(bin) => {
                if !matches!(bin.operator, BinaryOperator::Addition) {
                    return;
                }
                // CWE-400 guard: the chain helpers below (chain_has_string_evidence /
                // substitute_in_chain / fold_adjacent_literals) each recurse the ENTIRE
                // `+` chain, so process only at the chain ROOT. If this `+` node is
                // itself an operand of a parent `+` chain, skip it — the root resolves
                // the whole chain in one O(n) pass. Without this, exit_expression's
                // per-node recursive scans are O(n²) on a long `a+a+a+...` chain,
                // a CPU/hang DoS on hostile input.
                let parent_is_plus_chain = match ctx.parent() {
                    Ancestor::BinaryExpressionLeft(p) => matches!(*p.operator(), BinaryOperator::Addition),
                    Ancestor::BinaryExpressionRight(p) => matches!(*p.operator(), BinaryOperator::Addition),
                    _ => false,
                };
                if parent_is_plus_chain {
                    return;
                }
                let resolver_view: &Self = self;
                if !chain_has_string_evidence(&bin.left, resolver_view, ctx)
                    && !chain_has_string_evidence(&bin.right, resolver_view, ctx)
                {
                    return;
                }
                let subs_l = substitute_in_chain(&mut bin.left, self, ctx);
                let subs_r = substitute_in_chain(&mut bin.right, self, ctx);
                let folded = fold_adjacent_literals(expr, ctx);
                let total = subs_l + subs_r;
                if (total > 0 || folded) && self.rewrites < 10 {
                    deob_trace!("[AST/dyn-concat] inlined {total} ident(s) into '+' chain (folded={folded})");
                }
                self.rewrites = self.rewrites.saturating_add(total);
                self.changed |= total > 0 || folded;
            }
            Expression::TemplateLiteral(tmpl) => {
                let subs = substitute_in_template(tmpl, self, ctx);
                if subs > 0 && self.rewrites < 10 {
                    deob_trace!("[AST/dyn-concat] inlined {subs} ident(s) into template literal");
                }
                self.rewrites = self.rewrites.saturating_add(subs);
                self.changed |= subs > 0;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests;
