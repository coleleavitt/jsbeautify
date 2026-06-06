//! Scope-aware local slot renamer for Bun-minified bundles.
//!
//! Bun mangles function-local bindings to single characters (`H`, `$`, `q`,
//! `K`, `_`, …) that the export/class/param recovery passes leave untouched —
//! the dominant residual in a deobfuscated bundle (~hundreds of thousands of
//! tokens). This pass renames those single-char locals to readable per-scope
//! slot names (`v0`, `v1`, …).
//!
//! ## Correctness
//!
//! Renaming is keyed by [`SymbolId`], so every reference follows its binding
//! automatically (distinct bindings that happen to share the mangled name `H`
//! are distinct symbols and are renamed independently). To avoid introducing a
//! collision or wrongful shadow — which `node --check` cannot detect because it
//! is a *semantic* error, not a syntax one — each scope's slot names are chosen
//! to avoid every name visible in that scope: its own bindings, all ancestor
//! bindings, and the slot names already handed to ancestor scopes. Scopes are
//! processed outer-to-inner so ancestor assignments are known first.
//!
//! Bun names carry no original-name information, so this is *stabilisation*
//! (distinct, greppable, diff-stable names), not semantic recovery.

use oxc_ast::ast::{BindingIdentifier, Function, IdentifierReference};
use oxc_semantic::Scoping;
use oxc_syntax::scope::ScopeId;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;
pub type LocalRenameMap = FxHashMap<SymbolId, String>;

/// A name is a single-character Bun-mangled local: one ASCII letter, `$`, or `_`.
fn is_single_char_mangled(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() == 1 && (b[0].is_ascii_alphabetic() || b[0] == b'$' || b[0] == b'_')
}

/// Plan collision-free per-scope slot names for single-char mangled locals.
#[must_use]
pub fn plan_local_renames(scoping: &Scoping) -> LocalRenameMap {
    // Original binding names per scope (from every symbol's declaring scope).
    let mut scope_names: FxHashMap<ScopeId, Vec<(String, SymbolId)>> = FxHashMap::default();
    for sym in scoping.symbol_ids() {
        let scope = scoping.symbol_scope_id(sym);
        scope_names
            .entry(scope)
            .or_default()
            .push((scoping.symbol_name(sym).to_string(), sym));
    }

    // Scopes that contain at least one candidate, ordered outer-to-inner by depth
    // (ancestor count) so a scope is processed after all its ancestors.
    let mut scopes: Vec<(usize, ScopeId)> = scope_names
        .iter()
        .filter(|(_, names)| names.iter().any(|(n, _)| is_single_char_mangled(n)))
        .map(|(&sid, _)| (scoping.scope_ancestors(sid).count(), sid))
        .collect();
    scopes.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| format!("{:?}", a.1).cmp(&format!("{:?}", b.1)))
    });

    let mut plan: LocalRenameMap = FxHashMap::default();
    let mut assigned: FxHashMap<ScopeId, FxHashSet<String>> = FxHashMap::default();

    for (_depth, scope) in scopes {
        // Names that must not be reused in this scope.
        let mut blocked: FxHashSet<String> = FxHashSet::default();
        if let Some(names) = scope_names.get(&scope) {
            for (n, _) in names {
                blocked.insert(n.clone());
            }
        }
        for anc in scoping.scope_ancestors(scope) {
            if anc == scope {
                continue;
            }
            if let Some(names) = scope_names.get(&anc) {
                for (n, _) in names {
                    blocked.insert(n.clone());
                }
            }
            if let Some(a) = assigned.get(&anc) {
                blocked.extend(a.iter().cloned());
            }
        }

        // Candidates in this scope, deterministic order for stable output.
        let mut cands: Vec<(String, SymbolId)> = scope_names
            .get(&scope)
            .map(|names| {
                names
                    .iter()
                    .filter(|(n, _)| is_single_char_mangled(n))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        cands.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| format!("{:?}", a.1).cmp(&format!("{:?}", b.1)))
        });

        let mut local: FxHashSet<String> = FxHashSet::default();
        let mut counter = 0usize;
        for (_name, sym) in cands {
            let cand = loop {
                let c = format!("v{counter}");
                counter += 1;
                if !blocked.contains(&c) && !local.contains(&c) {
                    break c;
                }
            };
            local.insert(cand.clone());
            plan.insert(sym, cand);
        }
        if !local.is_empty() {
            assigned.insert(scope, local);
        }
    }

    plan
}

/// Applies a [`LocalRenameMap`] by `SymbolId` (binding decls + references).
pub struct LocalRenamer {
    map: LocalRenameMap,
    renamed: usize,
}

impl LocalRenamer {
    #[must_use]
    pub fn new(map: LocalRenameMap) -> Self {
        Self { map, renamed: 0 }
    }

    #[must_use]
    pub const fn renamed(&self) -> usize {
        self.renamed
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for LocalRenamer {
    fn exit_identifier_reference(&mut self, ident: &mut IdentifierReference<'a>, ctx: &mut Ctx<'a>) {
        let Some(symbol_id) = ctx.scoping().get_reference(ident.reference_id()).symbol_id() else {
            return;
        };
        if let Some(new_name) = self.map.get(&symbol_id)
            && ident.name.as_str() != new_name
        {
            ident.name = ctx.ast.ident(new_name);
        }
    }

    fn exit_binding_identifier(&mut self, ident: &mut BindingIdentifier<'a>, ctx: &mut Ctx<'a>) {
        let Some(symbol_id) = ident.symbol_id.get() else {
            return;
        };
        if let Some(new_name) = self.map.get(&symbol_id)
            && ident.name.as_str() != new_name
        {
            ident.name = ctx.ast.ident(new_name);
            self.renamed += 1;
        }
    }

    fn exit_function(&mut self, func: &mut Function<'a>, ctx: &mut Ctx<'a>) {
        if let Some(id) = &mut func.id
            && let Some(symbol_id) = id.symbol_id.get()
            && let Some(new_name) = self.map.get(&symbol_id)
            && id.name.as_str() != new_name
        {
            id.name = ctx.ast.ident(new_name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast_deobfuscate::DeobfuscateState;
    use oxc_allocator::Allocator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_semantic::SemanticBuilder;
    use oxc_span::SourceType;
    use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

    fn run(code: &str) -> String {
        let alloc = Allocator::default();
        let parsed = Parser::new(&alloc, code, SourceType::mjs()).parse();
        let mut prog = parsed.program;

        let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
        let plan = plan_local_renames(&scoping);

        let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &alloc);
        let mut renamer = LocalRenamer::new(plan);
        traverse_mut_with_ctx(&mut renamer, &mut prog, &mut ctx);
        Codegen::new().build(&prog).code
    }

    #[test]
    fn renames_single_char_locals_and_refs() {
        let out = run("function f(H, $) { let q = H + $; return q; }");
        // params + local are renamed and all references follow.
        assert!(
            out.contains("v0") && out.contains("v1") && out.contains("v2"),
            "got: {out}"
        );
        assert!(!out.contains("H +") && !out.contains("return q"), "stale refs: {out}");
    }

    #[test]
    fn sibling_scopes_reuse_slots_without_collision() {
        // Two independent sibling functions (multi-char names, so only the
        // single-char params are renamed): each param is a distinct symbol in a
        // non-overlapping scope, so both may safely take the same slot `v0`.
        let out = run("function alpha(H){return H} function beta(K){return K}");
        assert!(
            !out.contains("(H)") && !out.contains("(K)"),
            "params not renamed: {out}"
        );
        assert!(out.contains("alpha(v0)") && out.contains("beta(v0)"), "got: {out}");
    }

    #[test]
    fn does_not_shadow_outer_binding() {
        // Outer `q` (v0) is referenced inside inner scope; inner local `K` must
        // NOT be assigned v0 (which would shadow the outer reference).
        let out = run("function a(q){ return function(){ let K = 1; return q + K } }");
        // The inner `K` becomes a slot != the outer's slot; no name is reused
        // across the visible chain. Just assert it still references both.
        assert!(out.contains("return"), "got: {out}");
        // Outer q -> v0; inner K must be v1 (v0 blocked by ancestor).
        assert!(out.contains("v1"), "inner local should avoid ancestor slot: {out}");
    }
}
