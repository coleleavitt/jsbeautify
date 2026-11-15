use oxc_ast::ast::{Ident, IdentifierReference};
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::Traverse;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::{Ctx, ExportNameMap};
use crate::ast_deobfuscate::shorthand_targets::{expand_renamed_proto_shorthand, expand_renamed_shorthand};

/// Renames exported identifiers by resolved symbol, not by raw minified text.
pub struct ExportNameRenamer {
    name_map: ExportNameMap,
    renamed_refs: usize,
    renamed_decls: usize,
}

impl ExportNameRenamer {
    #[must_use]
    pub const fn new(name_map: ExportNameMap) -> Self {
        Self {
            name_map,
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

    #[must_use]
    pub const fn total_renamed(&self) -> usize {
        self.renamed_refs + self.renamed_decls
    }

    fn lookup(&self, symbol_id: SymbolId) -> Option<&str> {
        self.name_map.get(&symbol_id).map(String::as_str)
    }

    /// Whether this pass gives `binding` a new name.
    fn renames(&self, binding: &IdentifierReference<'_>, ctx: &Ctx<'_>) -> bool {
        binding
            .reference_id
            .get()
            .and_then(|reference| ctx.scoping().get_reference(reference).symbol_id())
            .and_then(|symbol| self.lookup(symbol))
            .is_some_and(|name| name != binding.name.as_str())
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for ExportNameRenamer {
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
        if let Some(original) = self.lookup(symbol_id)
            && ident.name.as_str() != original
        {
            ident.name = Ident::from_str_in(original, &ctx.ast);
            self.renamed_refs += 1;
        }
    }

    fn exit_binding_identifier(&mut self, ident: &mut oxc_ast::ast::BindingIdentifier<'a>, ctx: &mut Ctx<'a>) {
        let Some(symbol_id) = ident.symbol_id.get() else {
            return;
        };
        if let Some(original) = self.lookup(symbol_id)
            && ident.name.as_str() != original
        {
            ident.name = Ident::from_str_in(original, &ctx.ast);
            self.renamed_decls += 1;
        }
    }

    fn exit_function(&mut self, func: &mut oxc_ast::ast::Function<'a>, ctx: &mut Ctx<'a>) {
        if let Some(id) = &mut func.id {
            let Some(symbol_id) = id.symbol_id.get() else {
                return;
            };
            if let Some(original) = self.lookup(symbol_id)
                && id.name.as_str() != original
            {
                id.name = Ident::from_str_in(original, &ctx.ast);
                self.renamed_decls += 1;
            }
        }
    }
}
