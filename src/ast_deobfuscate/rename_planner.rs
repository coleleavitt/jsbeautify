//! Collision-free name assignment for the bundler rename plans (webpack,
//! Turbopack).
//!
//! Renaming symbol `S`, declared in scope `X`, to `N` is accepted only when:
//!
//! 1. no other binding named `N` is declared in `X` (no redeclaration);
//! 2. no reference inside `X`'s subtree resolves to an outer binding or global
//!    named `N` (it would be captured by `S`);
//! 3. no binding named `N` sits between a reference to `S` and `X` (it would
//!    capture that reference);
//! 4. for an Annex B block function, which `oxc_semantic` binds in the var
//!    scope `X`, no binding named `N` sits in the blocks from its declaration
//!    up to `X` (Annex B would then not create the `var`, or the declaration
//!    would collide).
//!
//! Otherwise `N$1`, `N$2`, … are tried. Names are tracked as they are
//! assigned, so later plan entries see earlier renames.

use oxc_allocator::Allocator;
use oxc_ast::ast::Program;
use oxc_semantic::Scoping;
use oxc_syntax::scope::ScopeId;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::ExportNameRenamer;
use crate::ast_deobfuscate::rename_guards::RenameGuardCollector;
use crate::ast_deobfuscate::state::DeobfuscateState;

/// Outcome of [`apply_rename_plan`].
pub struct RenameCounts {
    pub planned: usize,
    pub declarations: usize,
    pub references: usize,
}

/// Rename the symbols of `plan` (in priority order) to collision-free forms of
/// their desired names. Symbols the shared rename guards protect (with
/// `keep_fn_names`, also function-name bindings), and every symbol visible
/// from one of `with_scopes`, keep their names.
pub fn apply_rename_plan<'a>(
    allocator: &'a Allocator,
    program: &mut Program<'a>,
    mut ctx: ReusableTraverseCtx<'a, DeobfuscateState>,
    plan: Vec<(SymbolId, String)>,
    with_scopes: &[ScopeId],
    keep_fn_names: bool,
) -> RenameCounts {
    let mut guard = RenameGuardCollector::new()
        .keep_fn_names(keep_fn_names)
        .renaming_annex_b_functions();
    traverse_mut_with_ctx(&mut guard, program, &mut ctx);
    let scoping = ctx.into_scoping();
    let annex_b = guard.annex_b_blocks();
    let mut guarded = guard.into_guarded(&scoping);
    guarded.extend(symbols_visible_from(with_scopes, &scoping));

    let names = assign_names(plan, &guarded, &annex_b, &scoping);
    let planned = names.len();
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
    let mut renamer = ExportNameRenamer::new(names);
    traverse_mut_with_ctx(&mut renamer, program, &mut ctx);
    RenameCounts {
        planned,
        declarations: renamer.renamed_decls(),
        references: renamer.renamed_refs(),
    }
}

/// Every symbol visible from one of `scopes` (declared in it or an ancestor).
fn symbols_visible_from(scopes: &[ScopeId], scoping: &Scoping) -> FxHashSet<SymbolId> {
    if scopes.is_empty() {
        return FxHashSet::default();
    }
    let visible: FxHashSet<ScopeId> = scopes.iter().flat_map(|&s| scoping.scope_ancestors(s)).collect();
    scoping
        .symbol_ids()
        .filter(|&sym| visible.contains(&scoping.symbol_scope_id(sym)))
        .collect()
}

struct Planner<'s> {
    scoping: &'s Scoping,
    /// Base names of the plan; only these (and their `$N` forms) are tracked.
    wanted: FxHashSet<String>,
    /// `(scope, name)` for every binding declared in `scope`.
    bound: FxHashSet<(ScopeId, String)>,
    /// `(scope, name)` when a reference to `name` inside `scope` resolves to a
    /// binding outside it, or to a global.
    free: FxHashSet<(ScopeId, String)>,
    /// Annex B block functions and the blocks declaring them.
    annex_b: &'s FxHashMap<SymbolId, ScopeId>,
}

impl<'s> Planner<'s> {
    fn new(scoping: &'s Scoping, wanted: FxHashSet<String>, annex_b: &'s FxHashMap<SymbolId, ScopeId>) -> Self {
        let mut planner = Self {
            scoping,
            wanted,
            bound: FxHashSet::default(),
            free: FxHashSet::default(),
            annex_b,
        };
        for symbol in scoping.symbol_ids() {
            let name = scoping.symbol_name(symbol);
            if planner.is_tracked(name) {
                planner.declare(symbol, name);
            }
        }
        for (name, references) in scoping.root_unresolved_references() {
            if !planner.is_tracked(name) {
                continue;
            }
            for &reference in references {
                let scope = scoping.get_reference(reference).scope_id();
                for s in scoping.scope_ancestors(scope) {
                    planner.free.insert((s, name.to_string()));
                }
            }
        }
        planner
    }

    fn is_tracked(&self, name: &str) -> bool {
        self.wanted.contains(name)
            || name.rsplit_once('$').is_some_and(|(base, n)| {
                !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) && self.wanted.contains(base)
            })
    }

    fn reference_scopes(&self, symbol: SymbolId) -> impl Iterator<Item = ScopeId> + '_ {
        self.scoping
            .get_resolved_reference_ids(symbol)
            .iter()
            .map(|&reference| self.scoping.get_reference(reference).scope_id())
    }

    /// The blocks between an Annex B block function's declaration and its
    /// var scope `home`.
    fn annex_b_blocks(&self, symbol: SymbolId, home: ScopeId) -> Vec<ScopeId> {
        self.annex_b.get(&symbol).map_or_else(Vec::new, |&block| {
            self.scoping.scope_ancestors(block).take_while(|&s| s != home).collect()
        })
    }

    /// Record `symbol` as currently named `name`.
    fn declare(&mut self, symbol: SymbolId, name: &str) {
        let home = self.scoping.symbol_scope_id(symbol);
        self.bound.insert((home, name.to_string()));
        for block in self.annex_b_blocks(symbol, home) {
            self.bound.insert((block, name.to_string()));
        }
        let scopes: Vec<ScopeId> = self.reference_scopes(symbol).collect();
        for scope in scopes {
            for s in self.scoping.scope_ancestors(scope).take_while(|&s| s != home) {
                self.free.insert((s, name.to_string()));
            }
        }
    }

    fn is_safe(&self, symbol: SymbolId, name: &str) -> bool {
        let home = self.scoping.symbol_scope_id(symbol);
        let key = (home, name.to_string());
        if self.bound.contains(&key) || self.free.contains(&key) {
            return false;
        }
        if self
            .annex_b_blocks(symbol, home)
            .into_iter()
            .any(|block| self.bound.contains(&(block, name.to_string())))
        {
            return false;
        }
        self.reference_scopes(symbol).all(|scope| {
            self.scoping
                .scope_ancestors(scope)
                .take_while(|&s| s != home)
                .all(|s| !self.bound.contains(&(s, name.to_string())))
        })
    }

    fn assign(&mut self, symbol: SymbolId, desired: &str) -> String {
        let mut name = desired.to_string();
        let mut n = 1u32;
        while !self.is_safe(symbol, &name) {
            name = format!("{desired}${n}");
            n += 1;
        }
        self.declare(symbol, &name);
        name
    }
}

/// Resolve `plan` (in priority order) into final, collision-free names,
/// skipping `guarded` symbols and symbols already carrying the desired name.
fn assign_names(
    plan: Vec<(SymbolId, String)>,
    guarded: &FxHashSet<SymbolId>,
    annex_b: &FxHashMap<SymbolId, ScopeId>,
    scoping: &Scoping,
) -> FxHashMap<SymbolId, String> {
    let plan: Vec<_> = plan
        .into_iter()
        .filter(|(symbol, name)| !guarded.contains(symbol) && scoping.symbol_name(*symbol) != name)
        .collect();
    let mut planner = Planner::new(scoping, plan.iter().map(|(_, name)| name.clone()).collect(), annex_b);
    plan.into_iter()
        .map(|(symbol, desired)| (symbol, planner.assign(symbol, &desired)))
        .collect()
}

#[cfg(test)]
mod tests {
    use oxc_allocator::Allocator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_semantic::SemanticBuilder;
    use oxc_span::SourceType;
    use oxc_traverse::ReusableTraverseCtx;

    use super::apply_rename_plan;
    use crate::ast_deobfuscate::state::DeobfuscateState;

    /// Rename the first binding called `from` to `to` in sloppy `CommonJS` code.
    fn rename(code: &str, from: &str, to: &str) -> String {
        let allocator = Allocator::default();
        let mut program = Parser::new(&allocator, code, SourceType::cjs()).parse().program;
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let symbol = scoping
            .symbol_ids()
            .find(|&s| scoping.symbol_name(s) == from)
            .expect("binding");
        let ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        apply_rename_plan(
            &allocator,
            &mut program,
            ctx,
            vec![(symbol, to.to_string())],
            &[],
            false,
        );
        Codegen::new().build(&program).code
    }

    #[test]
    fn renames_an_annex_b_function_and_its_hoisted_var() {
        let out = rename("function g(x) { if (x) { function f() {} } return f; }", "f", "load");
        assert!(out.contains("function load() {}"), "{out}");
        assert!(out.contains("return load;"), "{out}");
    }

    #[test]
    fn keeps_an_annex_b_name_free_up_to_the_var_scope() {
        let out = rename(
            "function g(x) { { let load = 1; { function f() {} } } return f; }",
            "f",
            "load",
        );
        assert!(out.contains("function load$1() {}"), "{out}");
        assert!(out.contains("return load$1;"), "{out}");
    }

    #[test]
    fn keeps_an_annex_b_function_whose_var_is_another_binding() {
        let code = "function g(x) { var f = 1; { function f() {} } return f; }";
        assert!(rename(code, "f", "load").contains("function f() {}"));
    }
}
