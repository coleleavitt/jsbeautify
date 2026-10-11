//! Module wrapper handle renaming for the lazy `__esm` / `__commonJS`
//! wrappers that esbuild and Bun emit.

use oxc_ast::ast::{BindingIdentifier, BindingPattern, Expression, Ident, IdentifierReference, VariableDeclarator};
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::BunHelper;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;
pub type BunModuleHandleMap = FxHashMap<SymbolId, String>;

#[derive(Clone, Copy)]
enum BunModuleKind {
    Esm,
    CommonJs,
}

/// Finds `var x = __esm(...)` / `var x = __commonJS(...)` module handles, by
/// the resolved helper symbol of the callee.
pub struct BunModuleHandleCollector {
    helpers: FxHashMap<SymbolId, BunHelper>,
    handles: Vec<(SymbolId, BunModuleKind)>,
    occupied_names: FxHashSet<String>,
}

impl BunModuleHandleCollector {
    #[must_use]
    pub fn new(helpers: FxHashMap<SymbolId, BunHelper>) -> Self {
        Self {
            helpers,
            handles: Vec::new(),
            occupied_names: FxHashSet::default(),
        }
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.handles.is_empty()
    }

    /// Assigns `init_N` / `require_N` names that differ from every binding
    /// name and from `reserved`.
    #[must_use]
    pub fn into_map(mut self, reserved: &FxHashSet<String>) -> BunModuleHandleMap {
        self.occupied_names.extend(reserved.iter().cloned());
        let mut map = BunModuleHandleMap::default();
        let mut esm_count = 0;
        let mut cjs_count = 0;

        for (symbol_id, kind) in self.handles {
            let new_name = match kind {
                BunModuleKind::Esm => allocate_handle_name("init", &mut esm_count, &mut self.occupied_names),
                BunModuleKind::CommonJs => allocate_handle_name("require", &mut cjs_count, &mut self.occupied_names),
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

    fn enter_variable_declarator(&mut self, decl: &mut VariableDeclarator<'a>, ctx: &mut Ctx<'a>) {
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

        let helper = ctx
            .scoping()
            .get_reference(callee.reference_id())
            .symbol_id()
            .and_then(|symbol| self.helpers.get(&symbol));
        let kind = match helper {
            Some(BunHelper::Esm) => BunModuleKind::Esm,
            Some(BunHelper::CommonJs) => BunModuleKind::CommonJs,
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
    use crate::ast_deobfuscate::bun::BunRuntimeCollector;
    use oxc_allocator::Allocator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_semantic::SemanticBuilder;
    use oxc_span::SourceType;
    use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

    const HELPERS: &str = "var E = (e, t) => () => (e && (t = e(e = 0)), t);
        var C = (e, t) => () => (t || e((t = { exports: {} }).exports, t), t.exports);";

    fn run(code: &str) -> String {
        let code = format!("{HELPERS}\n{code}");
        let allocator = Allocator::default();
        let parsed = Parser::new(&allocator, &code, SourceType::mjs()).parse();
        let mut program = parsed.program;

        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        let mut helpers = BunRuntimeCollector::new();
        traverse_mut_with_ctx(&mut helpers, &mut program, &mut ctx);
        let mut collector = BunModuleHandleCollector::new(helpers.helper_symbols().clone());
        traverse_mut_with_ctx(&mut collector, &mut program, &mut ctx);
        let reserved = ctx
            .into_scoping()
            .root_unresolved_references()
            .keys()
            .map(ToString::to_string)
            .collect();
        let map = collector.into_map(&reserved);

        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        let mut renamer = BunModuleHandleRenamer::new(map);
        traverse_mut_with_ctx(&mut renamer, &mut program, &mut ctx);

        Codegen::new().build(&program).code
    }

    #[test]
    fn renames_module_handles_by_kind() {
        let out = run("var a = E(() => {}); var b = C((exports, module) => {}); a(); b();");
        assert!(out.contains("var init_1 = E("), "got: {out}");
        assert!(out.contains("var require_1 = C("), "got: {out}");
        assert!(out.contains("init_1();") && out.contains("require_1();"), "got: {out}");
    }

    #[test]
    fn avoids_existing_binding_and_global_names() {
        let out = run("var init_1 = 0; var a = E(() => {}); init_1; init_2; a();");
        assert!(out.contains("var init_1 = 0"), "existing binding should remain: {out}");
        assert!(
            out.contains("var init_3 = E("),
            "handle must skip the global init_2: {out}"
        );
        assert!(out.contains("init_2;\ninit_3();"), "got: {out}");
    }

    #[test]
    fn ignores_calls_to_a_shadowing_binding_with_the_helper_name() {
        let out = run("function f(E) { var a = E(() => {}); return a; }");
        assert!(out.contains("var a = E("), "local E is not the helper: {out}");
    }
}
