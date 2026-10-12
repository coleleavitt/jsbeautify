use oxc_ast::ast::{Expression, Ident};
use oxc_span::SPAN;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::Traverse;
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;
use super::utils::{GlobalObject, is_always_defined_global, is_delete_operand, is_reserved_word};

pub struct WindowAliasPropagator {
    aliases: FxHashMap<SymbolId, GlobalObject>,
    rewrites: usize,
    es5_only: bool,
}

impl WindowAliasPropagator {
    #[must_use]
    pub const fn new(aliases: FxHashMap<SymbolId, GlobalObject>) -> Self {
        Self {
            aliases,
            rewrites: 0,
            es5_only: false,
        }
    }

    /// Rewrite only the names ES5-era browsers define (see
    /// [`crate::Options::window_alias_es5`]).
    #[must_use]
    pub const fn es5_only(mut self, es5_only: bool) -> Self {
        self.es5_only = es5_only;
        self
    }

    #[must_use]
    pub const fn rewrites(&self) -> usize {
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
        let Some(&global) = self.aliases.get(&symbol_id) else {
            return;
        };
        let prop_name = sme.property.name.as_str();
        // `alias.in` → `in` would be a bare reserved word: invalid JS. Skip it.
        if is_reserved_word(prop_name) {
            return;
        }
        // Only rewrite properties that are *always defined* as global bindings
        // of the global object the alias holds. For an arbitrary `alias.foo`,
        // `window.foo` reads `undefined` when `foo` is absent, but bare `foo`
        // THROWS a ReferenceError — not behavior-preserving.
        if !is_always_defined_global(prop_name, global, self.es5_only) {
            return;
        }
        // `delete alias.foo` → `delete foo` is a SyntaxError in strict mode (MJS
        // is always strict). `delete` of a member is legal but `delete` of a bare
        // reference is not — never rewrite a `delete` operand (incl. through parens).
        if is_delete_operand(ctx) {
            return;
        }
        // Capture hazard: `alias.foo` → bare `foo` only stays equivalent to the
        // global property read if NO binding named `foo` is in scope here. If a
        // local/param/import `foo` is visible, the bare identifier would resolve
        // to *that* instead of `window.foo` — a silent miscompile. Skip those.
        if ctx
            .scoping()
            .find_binding(ctx.current_scope_id(), Ident::from_str_in(prop_name, ctx))
            .is_some()
        {
            return;
        }
        self.rewrites += 1;
        *expr = Expression::new_identifier(SPAN, prop_name, ctx);
    }
}
