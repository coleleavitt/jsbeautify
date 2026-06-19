use oxc_ast::ast::IdentifierReference;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::Traverse;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::{Ctx, ExportNameMap};

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
}

impl<'a> Traverse<'a, DeobfuscateState> for ExportNameRenamer {
    fn exit_identifier_reference(&mut self, ident: &mut IdentifierReference<'a>, ctx: &mut Ctx<'a>) {
        let Some(symbol_id) = ctx.scoping().get_reference(ident.reference_id()).symbol_id() else {
            return;
        };
        if let Some(original) = self.lookup(symbol_id)
            && ident.name.as_str() != original
        {
            ident.name = ctx.ast.ident(original);
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
            ident.name = ctx.ast.ident(original);
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
                id.name = ctx.ast.ident(original);
                self.renamed_decls += 1;
            }
        }
    }
}
