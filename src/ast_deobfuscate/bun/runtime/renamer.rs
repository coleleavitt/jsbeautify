use oxc_ast::ast::{BindingIdentifier, Ident, IdentifierReference};
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::Traverse;
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;
use crate::ast_deobfuscate::shorthand_targets::expand_renamed_shorthand;

/// Renames helper bindings and their resolved references to canonical names,
/// keyed by `SymbolId` so shadowing bindings with the same text are untouched.
pub struct BunRuntimeRenamer {
    map: FxHashMap<SymbolId, String>,
    renamed_refs: usize,
    renamed_decls: usize,
}

impl BunRuntimeRenamer {
    #[must_use]
    pub const fn new(map: FxHashMap<SymbolId, String>) -> Self {
        Self {
            map,
            renamed_refs: 0,
            renamed_decls: 0,
        }
    }

    #[must_use]
    pub const fn renamed_refs(&self) -> usize {
        self.renamed_refs
    }

    #[must_use]
    pub const fn renamed_decls(&self) -> usize {
        self.renamed_decls
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for BunRuntimeRenamer {
    fn enter_assignment_target_property(
        &mut self,
        prop: &mut oxc_ast::ast::AssignmentTargetProperty<'a>,
        ctx: &mut Ctx<'a>,
    ) {
        expand_renamed_shorthand(prop, ctx, |binding| {
            binding
                .reference_id
                .get()
                .and_then(|reference| ctx.scoping().get_reference(reference).symbol_id())
                .and_then(|symbol| self.map.get(&symbol))
                .is_some_and(|name| name != binding.name.as_str())
        });
    }

    fn exit_binding_identifier(&mut self, ident: &mut BindingIdentifier<'a>, ctx: &mut Ctx<'a>) {
        if let Some(symbol) = ident.symbol_id.get()
            && let Some(name) = self.map.get(&symbol)
            && ident.name.as_str() != name
        {
            ident.name = Ident::from_str_in(name, &ctx.ast);
            self.renamed_decls += 1;
        }
    }

    fn exit_identifier_reference(&mut self, ident: &mut IdentifierReference<'a>, ctx: &mut Ctx<'a>) {
        if let Some(symbol) = ctx.scoping().get_reference(ident.reference_id()).symbol_id()
            && let Some(name) = self.map.get(&symbol)
            && ident.name.as_str() != name
        {
            ident.name = Ident::from_str_in(name, &ctx.ast);
            self.renamed_refs += 1;
        }
    }
}
