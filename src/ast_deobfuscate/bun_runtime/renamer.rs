use oxc_ast::ast::{BindingPattern, Function, IdentifierReference, VariableDeclarator};
use oxc_traverse::Traverse;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;
use super::types::{BunHelper, BunHelperMap};

/// Renames mangled helper identifiers to their canonical names everywhere
/// in the program, using a map from `BunRuntimeCollector`.
pub struct BunRuntimeRenamer {
    map: BunHelperMap,
    renamed_refs: usize,
    renamed_decls: usize,
}

impl BunRuntimeRenamer {
    #[must_use]
    pub const fn new(map: BunHelperMap) -> Self {
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

    fn canonical_for(&self, mangled: &str) -> Option<&'static str> {
        self.map.get(mangled).map(BunHelper::canonical)
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for BunRuntimeRenamer {
    fn exit_identifier_reference(&mut self, ident: &mut IdentifierReference<'a>, ctx: &mut Ctx<'a>) {
        if let Some(canon) = self.canonical_for(ident.name.as_str()) {
            ident.name = ctx.ast.ident(canon);
            self.renamed_refs += 1;
        }
    }

    fn exit_variable_declarator(&mut self, decl: &mut VariableDeclarator<'a>, ctx: &mut Ctx<'a>) {
        // Rename `var V = …` → `var __esm = …`
        if let BindingPattern::BindingIdentifier(ident) = &mut decl.id
            && let Some(canon) = self.canonical_for(ident.name.as_str())
        {
            ident.name = ctx.ast.ident(canon);
            self.renamed_decls += 1;
        }
        // Rename inside `var { defineProperty: fjH, … } = Object;`
        if let BindingPattern::ObjectPattern(obj_pat) = &mut decl.id {
            for prop in &mut obj_pat.properties {
                if let BindingPattern::BindingIdentifier(b) = &mut prop.value
                    && let Some(canon) = self.canonical_for(b.name.as_str())
                {
                    b.name = ctx.ast.ident(canon);
                    self.renamed_decls += 1;
                }
            }
        }
    }

    fn exit_function(&mut self, func: &mut Function<'a>, ctx: &mut Ctx<'a>) {
        if let Some(id) = &mut func.id
            && let Some(canon) = self.canonical_for(id.name.as_str())
        {
            id.name = ctx.ast.ident(canon);
            self.renamed_decls += 1;
        }
    }
}
