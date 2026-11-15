use oxc_ast::ast::{BindingIdentifier, Function, Ident, IdentifierReference};
use oxc_traverse::Traverse;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::{Ctx, LocalRenameMap};
use crate::ast_deobfuscate::shorthand_targets::{expand_renamed_proto_shorthand, expand_renamed_shorthand};

/// Applies a [`LocalRenameMap`] by `SymbolId` (binding decls + references).
pub struct LocalRenamer {
    map: LocalRenameMap,
    renamed: usize,
    renamed_refs: usize,
    renamed_functions: usize,
}

impl LocalRenamer {
    #[must_use]
    pub const fn new(map: LocalRenameMap) -> Self {
        Self {
            map,
            renamed: 0,
            renamed_refs: 0,
            renamed_functions: 0,
        }
    }

    #[must_use]
    pub const fn renamed(&self) -> usize {
        self.renamed
    }

    #[must_use]
    pub const fn renamed_refs(&self) -> usize {
        self.renamed_refs
    }

    #[must_use]
    pub const fn renamed_functions(&self) -> usize {
        self.renamed_functions
    }

    #[must_use]
    pub const fn total_renamed(&self) -> usize {
        self.renamed + self.renamed_refs + self.renamed_functions
    }

    /// Whether this pass gives `binding` a new name.
    fn renames(&self, binding: &IdentifierReference<'_>, ctx: &Ctx<'_>) -> bool {
        binding
            .reference_id
            .get()
            .and_then(|reference| ctx.scoping().get_reference(reference).symbol_id())
            .and_then(|symbol| self.map.get(&symbol))
            .is_some_and(|name| name != binding.name.as_str())
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for LocalRenamer {
    fn enter_assignment_target_property(
        &mut self,
        prop: &mut oxc_ast::ast::AssignmentTargetProperty<'a>,
        ctx: &mut Ctx<'a>,
    ) {
        expand_renamed_shorthand(prop, ctx, |binding| self.renames(binding, ctx));
    }

    fn enter_object_property(&mut self, prop: &mut oxc_ast::ast::ObjectProperty<'a>, ctx: &mut Ctx<'a>) {
        expand_renamed_proto_shorthand(prop, ctx, |binding| self.renames(binding, ctx));
    }

    fn exit_identifier_reference(&mut self, ident: &mut IdentifierReference<'a>, ctx: &mut Ctx<'a>) {
        let Some(symbol_id) = ctx.scoping().get_reference(ident.reference_id()).symbol_id() else {
            return;
        };
        if let Some(new_name) = self.map.get(&symbol_id)
            && ident.name.as_str() != new_name
        {
            ident.name = Ident::from_str_in(new_name, &ctx.ast);
            self.renamed_refs += 1;
        }
    }

    fn exit_binding_identifier(&mut self, ident: &mut BindingIdentifier<'a>, ctx: &mut Ctx<'a>) {
        let Some(symbol_id) = ident.symbol_id.get() else {
            return;
        };
        if let Some(new_name) = self.map.get(&symbol_id)
            && ident.name.as_str() != new_name
        {
            ident.name = Ident::from_str_in(new_name, &ctx.ast);
            self.renamed += 1;
        }
    }

    fn exit_function(&mut self, func: &mut Function<'a>, ctx: &mut Ctx<'a>) {
        if let Some(id) = &mut func.id
            && let Some(symbol_id) = id.symbol_id.get()
            && let Some(new_name) = self.map.get(&symbol_id)
            && id.name.as_str() != new_name
        {
            id.name = Ident::from_str_in(new_name, &ctx.ast);
            self.renamed_functions += 1;
        }
    }
}
