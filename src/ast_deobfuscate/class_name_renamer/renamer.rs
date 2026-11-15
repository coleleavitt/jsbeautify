use oxc_ast::ast::IdentifierReference;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::Traverse;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::{ClassNameMap, Ctx};
use crate::ast_deobfuscate::shorthand_targets::expand_renamed_shorthand;

/// Renames class/function identifiers based on collected name maps.
pub struct ClassNameRenamer {
    name_map: ClassNameMap,
    renamed: usize,
}

impl ClassNameRenamer {
    #[must_use]
    pub const fn new(name_map: ClassNameMap) -> Self {
        Self { name_map, renamed: 0 }
    }

    #[must_use]
    pub const fn renamed(&self) -> usize {
        self.renamed
    }

    fn lookup(&self, symbol_id: SymbolId) -> Option<&str> {
        self.name_map.get(&symbol_id).map(String::as_str)
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for ClassNameRenamer {
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
                .and_then(|symbol| self.lookup(symbol))
                .is_some_and(|name| name != binding.name.as_str())
        });
    }

    fn exit_identifier_reference(&mut self, ident: &mut IdentifierReference<'a>, ctx: &mut Ctx<'a>) {
        let Some(symbol_id) = ctx.scoping().get_reference(ident.reference_id()).symbol_id() else {
            return;
        };
        if let Some(original) = self.lookup(symbol_id)
            && ident.name.as_str() != original
        {
            ident.name = oxc_ast::ast::Ident::from_str_in(original, ctx);
            self.renamed += 1;
        }
    }

    fn exit_binding_identifier(&mut self, ident: &mut oxc_ast::ast::BindingIdentifier<'a>, ctx: &mut Ctx<'a>) {
        let Some(symbol_id) = ident.symbol_id.get() else {
            return;
        };
        if let Some(original) = self.lookup(symbol_id)
            && ident.name.as_str() != original
        {
            ident.name = oxc_ast::ast::Ident::from_str_in(original, ctx);
            self.renamed += 1;
        }
    }
}
