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
//! to avoid every name that could capture a rewritten reference: its own
//! bindings, descendant bindings, all ancestor bindings, and the slot names
//! already handed to ancestor scopes. Scopes are processed outer-to-inner so
//! ancestor assignments are known first.
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

/// Idiomatic short names worth keeping even in readable mode — math/graphics/
/// loop identifiers a positional/usage rename would only make worse. Includes
/// `i`/`j`/`k` so loop-counter proposals are idempotent across re-runs.
const MEANINGFUL_SHORT: &[&str] = &[
    "i", "j", "k", "x", "y", "z", "w", "h", "id", "el", "db", "fs", "os", "fn", "cb", "ok", "ip", "ua", "px", "dx",
    "dy", "rgb", "rgba",
];

#[must_use]
pub fn is_meaningful_short(name: &str) -> bool {
    MEANINGFUL_SHORT.contains(&name)
}

/// Looks compiler/minifier-generated (esbuild/webpack/Bun aliases): one or two
/// bare letters, OR ≤3 letters with trailing digits, OR a `_`/`$` prefix.
/// Deliberately broad; [`is_meaningful_short`] is applied separately so
/// idiomatic short names survive.
#[must_use]
pub fn is_likely_generated_alias(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes.len() > 5 {
        return false;
    }
    let stripped = name.trim_start_matches(['_', '$']);
    let had_prefix = stripped.len() != name.len();
    if stripped.is_empty() {
        return name.len() <= 2; // "_", "$", "__"
    }
    let letters = stripped.chars().take_while(char::is_ascii_alphabetic).count();
    if letters == 0 {
        return false;
    }
    let rest = &stripped[letters..];
    if !rest.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    // 1-2 bare letters (`a`, `ab`).
    if letters <= 2 && rest.is_empty() && !had_prefix {
        return true;
    }
    // ≤3 letters WITH trailing digits or a `_`/`$` prefix (`ab1`, `_a2`, `$ref`).
    letters <= 3 && (!rest.is_empty() || had_prefix)
}

/// A base name usable as a readable identifier: valid JS identifier, not a
/// reserved word.
#[must_use]
pub fn is_usable_base(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_' || first == '$') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$') && !is_reserved_ident(name)
}

fn is_reserved_ident(name: &str) -> bool {
    matches!(
        name,
        "break"
            | "case"
            | "catch"
            | "class"
            | "const"
            | "continue"
            | "debugger"
            | "default"
            | "delete"
            | "do"
            | "else"
            | "export"
            | "extends"
            | "false"
            | "finally"
            | "for"
            | "function"
            | "if"
            | "import"
            | "in"
            | "instanceof"
            | "new"
            | "null"
            | "return"
            | "super"
            | "switch"
            | "this"
            | "throw"
            | "true"
            | "try"
            | "typeof"
            | "var"
            | "void"
            | "while"
            | "with"
            | "yield"
            | "let"
            | "static"
            | "enum"
            | "await"
            | "arguments"
            | "eval"
    )
}

/// First non-blocked variant of `base`: `base`, `base2`, … `base9`, else `None`.
fn pick_readable(base: &str, blocked: &FxHashSet<String>, local: &FxHashSet<String>) -> Option<String> {
    for n in 0..=9 {
        let cand = if n == 0 { base.to_string() } else { format!("{base}{n}") };
        if !blocked.contains(&cand) && !local.contains(&cand) {
            return Some(cand);
        }
    }
    None
}

/// How [`plan_local_renames`] chooses names.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RenameMode {
    /// Single-char locals → `v{n}` slots (the original stabilisation behaviour).
    Slot,
    /// Heuristic-proposed readable names ([`super::heuristic_rename`]) with a
    /// `v{n}` slot fallback; the candidate gate widens to all generated aliases.
    Readable,
    /// Authoritative names (sourcemap / LLM): rename ONLY symbols with a valid,
    /// different `desired` name — never slot, never touch unknown bindings.
    Sourcemap,
}

/// Plan collision-free, scope-safe rename names for minified locals.
///
/// - `exclude`: soundness-guarded symbols ([`super::rename_guards`]) — never
///   renamed, but still reserve their original name against collisions.
/// - `mode`: see [`RenameMode`] — slot stabilisation, heuristic readable names,
///   or authoritative sourcemap/LLM names.
/// - `desired`: proposed names keyed by `SymbolId` (heuristic / sourcemap / LLM);
///   ignored in [`RenameMode::Slot`].
/// - `free_names`: per-scope UNRESOLVED (free/global) reference names — a
///   readable name is blocked in any scope whose subtree references that name as
///   a free variable, so a rename can never *capture* a global (e.g. naming a
///   local `crypto` next to a free `crypto`).
#[must_use]
pub fn plan_local_renames(
    scoping: &Scoping,
    exclude: &FxHashSet<SymbolId>,
    mode: RenameMode,
    desired: &LocalRenameMap,
    free_names: &FxHashMap<ScopeId, FxHashSet<String>>,
) -> LocalRenameMap {
    // Original binding names per scope (from every symbol's declaring scope).
    let mut scope_names: FxHashMap<ScopeId, Vec<(String, SymbolId)>> = FxHashMap::default();
    let mut subtree_names: FxHashMap<ScopeId, FxHashSet<String>> = FxHashMap::default();
    for sym in scoping.symbol_ids() {
        let scope = scoping.symbol_scope_id(sym);
        let name = scoping.symbol_name(sym).to_string();
        scope_names.entry(scope).or_default().push((name.clone(), sym));
        for visible in scoping.scope_ancestors(scope) {
            subtree_names.entry(visible).or_default().insert(name.clone());
        }
    }

    // Free (unresolved) reference names per scope SUBTREE: propagate each free
    // name up to all ancestor scopes (a binding is visible to its descendants,
    // so a rename in scope S can capture a free reference anywhere under S).
    let mut free_subtree: FxHashMap<ScopeId, FxHashSet<String>> = FxHashMap::default();
    for (&scope, names) in free_names {
        for anc in scoping.scope_ancestors(scope) {
            let bucket = free_subtree.entry(anc).or_default();
            for n in names {
                bucket.insert(n.clone());
            }
        }
    }

    // A symbol is a rename candidate only if it looks minifier-generated (gate
    // widens in readable mode) AND is not soundness-excluded.
    let is_candidate = |name: &str, sym: SymbolId| {
        if exclude.contains(&sym) {
            return false;
        }
        // Already at its desired name (loop var already `i`, sourcemap name
        // already applied): leave it. Without this, re-running drifts
        // (`i`→`i1`) because the binding's own name sits in the blocked set.
        if desired.get(&sym).map(String::as_str) == Some(name) {
            return false;
        }
        match mode {
            RenameMode::Slot => is_single_char_mangled(name),
            // A meaningful-short name (`h`, `i`, …) is preserved UNLESS a
            // heuristic proposed a concrete name for it (`h = new
            // URLSearchParams()` → `urlSearchParams` beats the "height" guess).
            RenameMode::Readable => {
                is_likely_generated_alias(name) && (!is_meaningful_short(name) || desired.contains_key(&sym))
            }
            // Sourcemap is authoritative: rename any binding it names, minified
            // or not, as long as the original name is a usable identifier.
            RenameMode::Sourcemap => desired.get(&sym).is_some_and(|d| is_usable_base(d)),
        }
    };

    // Scopes that contain at least one candidate, ordered outer-to-inner by depth
    // (ancestor count) so a scope is processed after all its ancestors.
    let mut scopes: Vec<(usize, ScopeId)> = scope_names
        .iter()
        .filter(|(_, names)| names.iter().any(|(n, s)| is_candidate(n, *s)))
        .map(|(&sid, _)| (scoping.scope_ancestors(sid).count(), sid))
        .collect();
    scopes.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| format!("{:?}", a.1).cmp(&format!("{:?}", b.1)))
    });

    let mut plan: LocalRenameMap = FxHashMap::default();
    let mut assigned: FxHashMap<ScopeId, FxHashSet<String>> = FxHashMap::default();

    for (_depth, scope) in scopes {
        // Names that must not be reused in this scope. Descendant bindings are
        // included because references to this symbol may be emitted inside those
        // scopes after renaming; choosing a descendant's existing name would be
        // captured by that descendant binding at runtime.
        let mut blocked: FxHashSet<String> = FxHashSet::default();
        if let Some(names) = subtree_names.get(&scope) {
            blocked.extend(names.iter().cloned());
        }
        // Capture safety: never pick a readable name that matches a free/global
        // reference used in this scope's subtree.
        if let Some(free) = free_subtree.get(&scope) {
            blocked.extend(free.iter().cloned());
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
            .map(|names| names.iter().filter(|(n, s)| is_candidate(n, *s)).cloned().collect())
            .unwrap_or_default();
        cands.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| format!("{:?}", a.1).cmp(&format!("{:?}", b.1)))
        });

        let mut local: FxHashSet<String> = FxHashSet::default();
        let mut counter = 0usize;
        for (cur_name, sym) in cands {
            // A binding may always reuse its OWN current name (it cannot capture
            // itself), so lift it out of `blocked` for this choice. Without this,
            // re-running drifts (`v0`→`v1`, `i`→`i1`) because the binding's own
            // name sits in the subtree-blocked set.
            let self_blocked = blocked.remove(&cur_name);
            // Readable/Sourcemap: try the proposed base (and base2…base9).
            // Readable/Slot then fall back to the next `v{n}` slot; Sourcemap
            // does NOT slot — if its name is fully blocked, the binding is left
            // unchanged (skipped).
            let proposed = (mode != RenameMode::Slot)
                .then(|| desired.get(&sym))
                .flatten()
                .filter(|b| is_usable_base(b))
                .and_then(|base| pick_readable(base, &blocked, &local));
            let cand = match (proposed, mode) {
                (Some(name), _) => Some(name),
                (None, RenameMode::Sourcemap) => None, // leave unchanged
                (None, _) => Some(loop {
                    let c = format!("v{counter}");
                    counter += 1;
                    if !blocked.contains(&c) && !local.contains(&c) {
                        break c;
                    }
                }),
            };
            if self_blocked {
                blocked.insert(cur_name);
            }
            if let Some(cand) = cand {
                local.insert(cand.clone());
                plan.insert(sym, cand);
            }
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
        use crate::ast_deobfuscate::rename_guards::RenameGuardCollector;

        let alloc = Allocator::default();
        let parsed = Parser::new(&alloc, code, SourceType::mjs()).parse();
        let mut prog = parsed.program;

        // Soundness guard (mirrors the Phase 12.5 wiring in `mod.rs`).
        let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &alloc);
        let mut guard = RenameGuardCollector::new();
        traverse_mut_with_ctx(&mut guard, &mut prog, &mut ctx);
        drop(ctx);

        let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
        let guarded = guard.into_guarded(&scoping);
        // Slot mode (the original behaviour these tests pin): no proposals.
        let plan = plan_local_renames(
            &scoping,
            &guarded,
            RenameMode::Slot,
            &FxHashMap::default(),
            &FxHashMap::default(),
        );

        let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &alloc);
        let mut renamer = LocalRenamer::new(plan);
        traverse_mut_with_ctx(&mut renamer, &mut prog, &mut ctx);
        Codegen::new().build(&prog).code
    }

    /// Readable mode: run the heuristic collector + capture-safe planner.
    fn run_readable(code: &str) -> String {
        use crate::ast_deobfuscate::heuristic_rename::HeuristicNameCollector;
        use crate::ast_deobfuscate::rename_guards::RenameGuardCollector;

        let alloc = Allocator::default();
        let parsed = Parser::new(&alloc, code, SourceType::mjs()).parse();
        let mut prog = parsed.program;

        let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &alloc);
        let mut guard = RenameGuardCollector::new();
        traverse_mut_with_ctx(&mut guard, &mut prog, &mut ctx);
        drop(ctx);

        let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
        let guarded = guard.into_guarded(&scoping);

        let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &alloc);
        let mut collector = HeuristicNameCollector::new();
        traverse_mut_with_ctx(&mut collector, &mut prog, &mut ctx);
        drop(ctx);
        let (desired, free_names) = collector.into_parts();

        let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
        let plan = plan_local_renames(&scoping, &guarded, RenameMode::Readable, &desired, &free_names);

        let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &alloc);
        let mut renamer = LocalRenamer::new(plan);
        traverse_mut_with_ctx(&mut renamer, &mut prog, &mut ctx);
        Codegen::new().build(&prog).code
    }

    #[test]
    fn readable_infers_loop_catch_require_new() {
        let out = run_readable(
            "function f(){ for(let a=0;a<n;a++){g(a)} try{h()}catch(b){j(b)} var c=require('crypto'); var d=new Map(); }",
        );
        assert!(out.contains("let i = 0") && out.contains("i < n"), "loop: {out}");
        assert!(
            out.contains("catch (error)") && out.contains("j(error)"),
            "catch: {out}"
        );
        assert!(out.contains("crypto = require("), "require: {out}");
        assert!(out.contains("map = new Map("), "new: {out}");
    }

    #[test]
    fn readable_rename_never_captures_a_free_global() {
        // `a` would be named `crypto` (require heuristic), but a FREE `crypto`
        // global is referenced in the same scope — naming the local `crypto`
        // would capture that reference. It must fall back instead.
        let out = run_readable("function f(){ var a = require('crypto'); return crypto.subtle.digest(a); }");
        // The free global `crypto` reference must remain a free `crypto`, and the
        // local must NOT have been named `crypto` (which would shadow it).
        assert!(out.contains("crypto.subtle"), "free global ref lost: {out}");
        assert!(
            !out.contains("var crypto = require"),
            "local captured the free global crypto: {out}"
        );
    }

    #[test]
    fn readable_preserves_idiomatic_short_names() {
        // `x`/`y` are allowlisted (coordinates) and have no proposal → kept.
        let out = run_readable("function dist(x, y){ return Math.sqrt(x*x + y*y) }");
        assert!(out.contains("dist(x, y)"), "idiomatic shorts clobbered: {out}");
    }

    #[test]
    fn readable_is_idempotent() {
        let code = "function f(){ for(let a=0;a<n;a++){g(a)} try{x()}catch(b){j(b)} }";
        let once = run_readable(code);
        let twice = run_readable(&once);
        assert_eq!(once, twice, "second pass changed output:\n{once}\n---\n{twice}");
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

    #[test]
    fn outer_slot_avoids_existing_descendant_binding() {
        // If outer `H` became `v0`, the reference inside the nested function
        // would bind to the inner `let v0` after codegen.
        let out = run("function f(H){ return function(){ let v0 = 1; return H + v0; } }");
        assert!(
            !out.contains("return v0 + v0") && !out.contains("return v0+v0"),
            "outer reference was captured by descendant binding: {out}"
        );
    }

    #[test]
    fn does_not_rename_locals_visible_to_direct_eval() {
        // A direct `eval` resolves `H` by name; renaming it would dangle the
        // reference inside the eval string and break at runtime.
        let out = run("function f(H){ return eval('H + 1'); }");
        assert!(out.contains("(H)"), "eval-visible param must stay H: {out}");
        assert!(out.contains("eval(\"H + 1\")"), "eval string must be intact: {out}");
        assert!(!out.contains("v0"), "no local should be renamed: {out}");
    }

    #[test]
    fn does_not_rename_native_export_bindings() {
        // The binding name IS the public export name; renaming it would change
        // the module's public surface (importers break).
        let out = run("export function H(){ return 1 } export const K = 2;");
        assert!(out.contains("function H("), "exported fn name must be preserved: {out}");
        assert!(
            out.contains("K = 2") || out.contains("K=2"),
            "exported const must stay K: {out}"
        );
        assert!(
            !out.contains("v0") && !out.contains("v1"),
            "exports must not be renamed: {out}"
        );
    }

    #[test]
    fn still_renames_default_export_local() {
        // `export default function H(){}` exposes only `default`, so the local
        // name `H` is free to be renamed.
        let out = run("export default function H(){ let q = 1; return q }");
        assert!(
            !out.contains("function H("),
            "default export local should rename: {out}"
        );
        assert!(out.contains("export default function v0"), "got: {out}");
    }
}
