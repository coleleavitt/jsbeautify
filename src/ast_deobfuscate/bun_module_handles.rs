//! Bun module wrapper handle renaming.

use oxc_ast::ast::{BindingIdentifier, BindingPattern, Expression, Ident, IdentifierReference, VariableDeclarator};
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;
pub type BunModuleHandleMap = FxHashMap<SymbolId, String>;

#[derive(Clone, Copy)]
enum BunModuleKind {
    Esm,
    CommonJs,
}

#[derive(Default)]
pub struct BunModuleHandleCollector {
    handles: Vec<(SymbolId, BunModuleKind)>,
    occupied_names: FxHashSet<String>,
}

impl BunModuleHandleCollector {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn into_map(mut self) -> BunModuleHandleMap {
        let mut map = BunModuleHandleMap::default();
        let mut esm_count = 0;
        let mut cjs_count = 0;

        for (symbol_id, kind) in self.handles {
            let new_name = match kind {
                BunModuleKind::Esm => allocate_handle_name("bun_init", &mut esm_count, &mut self.occupied_names),
                BunModuleKind::CommonJs => {
                    allocate_handle_name("bun_require", &mut cjs_count, &mut self.occupied_names)
                }
            };
            map.entry(symbol_id).or_insert(new_name);
        }

        map
    }
}

fn allocate_handle_name(prefix: &str, counter: &mut usize, occupied_names: &mut FxHashSet<String>) -> String {
    loop {
        *counter += 1;
        let candidate = format!("{prefix}_{counter}");
        if occupied_names.insert(candidate.clone()) {
            return candidate;
        }
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for BunModuleHandleCollector {
    fn enter_binding_identifier(&mut self, ident: &mut BindingIdentifier<'a>, _ctx: &mut Ctx<'a>) {
        self.occupied_names.insert(ident.name.to_string());
    }

    fn enter_variable_declarator(&mut self, decl: &mut VariableDeclarator<'a>, _ctx: &mut Ctx<'a>) {
        let BindingPattern::BindingIdentifier(binding) = &decl.id else {
            return;
        };
        let Some(symbol_id) = binding.symbol_id.get() else {
            return;
        };
        let Some(Expression::CallExpression(call)) = &decl.init else {
            return;
        };
        let Expression::Identifier(callee) = &call.callee else {
            return;
        };

        let kind = match callee.name.as_str() {
            "__esm" => BunModuleKind::Esm,
            "__commonJS" => BunModuleKind::CommonJs,
            _ => return,
        };

        self.handles.push((symbol_id, kind));
    }
}

pub struct BunModuleHandleRenamer {
    map: BunModuleHandleMap,
    renamed_decls: usize,
    renamed_refs: usize,
}

impl BunModuleHandleRenamer {
    #[must_use]
    pub const fn new(map: BunModuleHandleMap) -> Self {
        Self {
            map,
            renamed_decls: 0,
            renamed_refs: 0,
        }
    }

    #[must_use]
    pub const fn renamed_decls(&self) -> usize {
        self.renamed_decls
    }

    #[must_use]
    pub const fn renamed_refs(&self) -> usize {
        self.renamed_refs
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for BunModuleHandleRenamer {
    fn exit_binding_identifier(&mut self, ident: &mut oxc_ast::ast::BindingIdentifier<'a>, ctx: &mut Ctx<'a>) {
        let Some(symbol_id) = ident.symbol_id.get() else {
            return;
        };
        let Some(new_name) = self.map.get(&symbol_id) else {
            return;
        };
        if ident.name.as_str() != new_name {
            ident.name = Ident::from_str_in(new_name, &ctx.ast);
            self.renamed_decls += 1;
        }
    }

    fn exit_identifier_reference(&mut self, ident: &mut IdentifierReference<'a>, ctx: &mut Ctx<'a>) {
        let Some(symbol_id) = ctx.scoping().get_reference(ident.reference_id()).symbol_id() else {
            return;
        };
        let Some(new_name) = self.map.get(&symbol_id) else {
            return;
        };
        if ident.name.as_str() != new_name {
            ident.name = Ident::from_str_in(new_name, &ctx.ast);
            self.renamed_refs += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_allocator::Allocator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_semantic::SemanticBuilder;
    use oxc_span::SourceType;
    use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

    fn run(code: &str) -> String {
        let allocator = Allocator::default();
        let parsed = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let mut program = parsed.program;

        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        let mut collector = BunModuleHandleCollector::new();
        traverse_mut_with_ctx(&mut collector, &mut program, &mut ctx);
        let map = collector.into_map();

        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        let mut renamer = BunModuleHandleRenamer::new(map);
        traverse_mut_with_ctx(&mut renamer, &mut program, &mut ctx);

        Codegen::new().build(&program).code
    }

    #[test]
    fn renames_bun_module_handles_by_kind() {
        let out = run("var a = __esm(() => {}); var b = __commonJS((exports, module) => {}); a(); b();");
        assert!(out.contains("bun_init_1"), "got: {out}");
        assert!(out.contains("bun_require_1"), "got: {out}");
        assert!(!out.contains("a();"), "init handle ref should rename: {out}");
        assert!(!out.contains("b();"), "require handle ref should rename: {out}");
    }

    #[test]
    fn avoids_existing_handle_name_collisions() {
        let out = run("var bun_init_1 = 0; var a = __esm(() => {}); bun_init_1; a();");
        assert!(
            out.contains("var bun_init_1 = 0"),
            "existing binding should remain: {out}"
        );
        assert!(
            out.contains("bun_init_2 = __esm"),
            "handle should get a free name: {out}"
        );
        assert!(out.contains("bun_init_2();"), "handle ref should use free name: {out}");
    }
}
