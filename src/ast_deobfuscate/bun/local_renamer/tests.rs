use super::*;
use crate::ast_deobfuscate::rename_apply::SymbolRenamer;
use crate::ast_deobfuscate::rename_guards::RenameGuardCollector;
use crate::ast_deobfuscate::{DeobfuscateState, semantic};
#[cfg(test)]
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::Scoping;
use oxc_span::SourceType;
use oxc_syntax::scope::ScopeId;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};
use rustc_hash::FxHashSet;

fn run(code: &str) -> String {
    let alloc = Allocator::default();
    let parsed = Parser::new(&alloc, code, SourceType::mjs()).parse();
    let mut prog = parsed.program;

    // Soundness guard (mirrors the Phase 12.5 wiring in `mod.rs`).
    let scoping = semantic::scoping(&prog);
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &alloc);
    let mut guard = RenameGuardCollector::new();
    traverse_mut_with_ctx(&mut guard, &mut prog, &mut ctx);
    drop(ctx);

    let scoping = semantic::scoping(&prog);
    let guarded = guard.into_guarded(&scoping);
    // Slot mode (the original behaviour these tests pin): no proposals.
    let plan = plan_local_renames(
        &scoping,
        &guarded,
        RenameMode::Slot,
        &FxHashMap::default(),
        &FxHashMap::default(),
    );

    let scoping = semantic::scoping(&prog);
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &alloc);
    let mut renamer = SymbolRenamer::new(plan);
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

    let scoping = semantic::scoping(&prog);
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &alloc);
    let mut guard = RenameGuardCollector::new();
    traverse_mut_with_ctx(&mut guard, &mut prog, &mut ctx);
    drop(ctx);

    let scoping = semantic::scoping(&prog);
    let guarded = guard.into_guarded(&scoping);

    let scoping = semantic::scoping(&prog);
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &alloc);
    let mut collector = HeuristicNameCollector::new();
    traverse_mut_with_ctx(&mut collector, &mut prog, &mut ctx);
    drop(ctx);
    let (desired, free_names) = collector.into_parts();

    let scoping = semantic::scoping(&prog);
    let plan = plan_local_renames(&scoping, &guarded, RenameMode::Readable, &desired, &free_names);

    let scoping = semantic::scoping(&prog);
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &alloc);
    let mut renamer = SymbolRenamer::new(plan);
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

/// Slot-mode plan + stats for `code` (mirrors the Phase 12.5 guard wiring).
fn plan_with_stats(code: &str) -> (LocalRenameMap, LocalRenamePlanStats) {
    use crate::ast_deobfuscate::rename_guards::RenameGuardCollector;

    let alloc = Allocator::default();
    let parsed = Parser::new(&alloc, code, SourceType::mjs()).parse();
    let mut prog = parsed.program;

    let scoping = semantic::scoping(&prog);
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &alloc);
    let mut guard = RenameGuardCollector::new();
    traverse_mut_with_ctx(&mut guard, &mut prog, &mut ctx);
    drop(ctx);

    let scoping = semantic::scoping(&prog);
    let guarded = guard.into_guarded(&scoping);
    plan_local_renames_with_stats(
        &scoping,
        &guarded,
        RenameMode::Slot,
        &FxHashMap::default(),
        &FxHashMap::default(),
    )
}

#[test]
fn stats_count_planned_symbols_and_match_plan_len() {
    // Two renameable single-char locals (`a`, `b`) in one function.
    let (plan, stats) = plan_with_stats("function f(){ let a = 1; let b = 2; return a + b; }");
    assert_eq!(
        plan.len(),
        stats.planned_symbols,
        "planned_symbols must equal plan size"
    );
    assert!(
        stats.planned_symbols >= 2,
        "expected >=2 planned, got {}",
        stats.planned_symbols
    );
    // The scoping counts are populated (non-zero) for a non-empty program.
    assert!(
        stats.symbols > 0 && stats.scopes > 0 && stats.references > 0,
        "stats: {stats:?}"
    );
}

#[test]
fn stats_plan_agrees_with_plain_planner() {
    // The _with_stats variant must produce the exact same plan as the plain
    // wrapper (which just discards the stats).
    let code = "function f(){ let a = 1; function g(b){ return b + a; } return g(2); }";
    let (plan_stats, _) = plan_with_stats(code);

    let alloc = Allocator::default();
    let parsed = Parser::new(&alloc, code, SourceType::mjs()).parse();
    let mut prog = parsed.program;
    let scoping = semantic::scoping(&prog);
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &alloc);
    let mut guard = RenameGuardCollector::new();
    traverse_mut_with_ctx(&mut guard, &mut prog, &mut ctx);
    drop(ctx);
    let scoping = semantic::scoping(&prog);
    let guarded = guard.into_guarded(&scoping);
    let plain = plan_local_renames(
        &scoping,
        &guarded,
        RenameMode::Slot,
        &FxHashMap::default(),
        &FxHashMap::default(),
    );
    assert_eq!(
        plan_stats.len(),
        plain.len(),
        "stats variant + plain wrapper must plan identically"
    );
}

#[test]
fn stats_empty_program_plans_nothing() {
    let (plan, stats) = plan_with_stats("");
    assert!(plan.is_empty());
    assert_eq!(stats.planned_symbols, 0);
}

/// The planner before it was made near-linear: walks every ancestor per
/// symbol and per probe. Kept as the oracle the fast planner must match.
fn reference_plan(
    scoping: &Scoping,
    exclude: &FxHashSet<SymbolId>,
    mode: RenameMode,
    desired: &LocalRenameMap,
    free_names: &FxHashMap<ScopeId, FxHashSet<String>>,
) -> LocalRenameMap {
    let mut scope_names: FxHashMap<ScopeId, Vec<(&str, SymbolId)>> = FxHashMap::default();
    let mut scope_name_sets: FxHashMap<ScopeId, FxHashSet<&str>> = FxHashMap::default();
    let mut subtree: FxHashMap<ScopeId, FxHashSet<&str>> = FxHashMap::default();
    for sym in scoping.symbol_ids() {
        let scope = scoping.symbol_scope_id(sym);
        let name = scoping.symbol_name(sym);
        scope_names.entry(scope).or_default().push((name, sym));
        scope_name_sets.entry(scope).or_default().insert(name);
        for anc in scoping.scope_ancestors(scope) {
            subtree.entry(anc).or_default().insert(name);
        }
    }
    for (&scope, names) in free_names {
        for anc in scoping.scope_ancestors(scope) {
            subtree.entry(anc).or_default().extend(names.iter().map(String::as_str));
        }
    }
    let is_candidate = |name: &str, sym: SymbolId| {
        if exclude.contains(&sym) || desired.get(&sym).map(String::as_str) == Some(name) {
            return false;
        }
        match mode {
            RenameMode::Slot => names::is_single_char_mangled(name),
            RenameMode::Readable => {
                is_likely_generated_alias(name) && (!is_meaningful_short(name) || desired.contains_key(&sym))
            }
            RenameMode::Sourcemap => desired.get(&sym).is_some_and(|d| is_usable_base(d)),
        }
    };
    let mut scopes: Vec<(usize, ScopeId)> = scope_names
        .iter()
        .filter(|(_, names)| names.iter().any(|(n, s)| is_candidate(n, *s)))
        .map(|(&sid, _)| (scoping.scope_ancestors(sid).count(), sid))
        .collect();
    scopes.sort_by_key(|&(depth, sid)| (depth, sid.index()));

    let mut plan = LocalRenameMap::default();
    let mut assigned: FxHashMap<ScopeId, FxHashSet<String>> = FxHashMap::default();
    for (_, scope) in scopes {
        let ancestors: Vec<ScopeId> = scoping.scope_ancestors(scope).skip(1).collect();
        let mut cands: Vec<(&str, SymbolId)> = scope_names[&scope]
            .iter()
            .filter(|(n, s)| is_candidate(n, *s))
            .copied()
            .collect();
        cands.sort_by(|a, b| a.0.cmp(b.0).then_with(|| a.1.index().cmp(&b.1.index())));
        let mut local: FxHashSet<String> = FxHashSet::default();
        let mut counter = 0usize;
        for (cur, sym) in cands {
            let blocked = |name: &str, local: &FxHashSet<String>| {
                if name == cur {
                    return local.contains(name);
                }
                local.contains(name)
                    || subtree.get(&scope).is_some_and(|s| s.contains(name))
                    || ancestors.iter().any(|anc| {
                        scope_name_sets.get(anc).is_some_and(|s| s.contains(name))
                            || assigned.get(anc).is_some_and(|s| s.contains(name))
                    })
            };
            let proposed = (mode != RenameMode::Slot)
                .then(|| desired.get(&sym))
                .flatten()
                .filter(|b| is_usable_base(b))
                .and_then(|base| names::pick_readable(base, |c| blocked(c, &local)));
            let cand = match (proposed, mode) {
                (Some(name), _) => Some(name),
                (None, RenameMode::Sourcemap) => None,
                (None, _) => Some(loop {
                    let c = format!("v{counter}");
                    counter += 1;
                    if !blocked(&c, &local) {
                        break c;
                    }
                }),
            };
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

/// Free (unresolved) reference names by scope, as Phase 12.5 collects them.
fn free_names_of(scoping: &Scoping) -> FxHashMap<ScopeId, FxHashSet<String>> {
    let mut names: FxHashMap<ScopeId, FxHashSet<String>> = FxHashMap::default();
    for (name, references) in scoping.root_unresolved_references() {
        for &reference in references {
            let scope = scoping.get_reference(reference).scope_id();
            names.entry(scope).or_default().insert(name.to_string());
        }
    }
    names
}

/// Plans `code` in every mode with both planners and asserts the plans match.
/// The desired names are drawn from a small pool so they collide with slot
/// names, with each other and with the bindings' own names.
fn assert_matches_reference(code: &str) {
    const POOL: &[&str] = &["v0", "v1", "v2", "v10", "a", "b", "x", "v1", "q"];
    let alloc = Allocator::default();
    let parsed = Parser::new(&alloc, code, SourceType::mjs()).parse();
    assert!(parsed.diagnostics.is_empty(), "test input must parse: {code}");
    let scoping = semantic::scoping(&parsed.program);
    let free = free_names_of(&scoping);
    let desired: LocalRenameMap = scoping
        .symbol_ids()
        .enumerate()
        .filter(|(i, _)| i % 4 != 3)
        .map(|(i, sym)| (sym, POOL[(i * 7) % POOL.len()].to_string()))
        .collect();
    let exclude: FxHashSet<SymbolId> = scoping.symbol_ids().step_by(5).collect();
    for mode in [RenameMode::Slot, RenameMode::Readable, RenameMode::Sourcemap] {
        for exclude in [&FxHashSet::default(), &exclude] {
            let fast = plan_local_renames(&scoping, exclude, mode, &desired, &free);
            let slow = reference_plan(&scoping, exclude, mode, &desired, &free);
            assert_eq!(fast, slow, "{mode:?} plan differs for {code}");
        }
    }
}

#[test]
fn planner_matches_reference_on_shadowing_and_slot_name_inputs() {
    for code in [
        "function f(H){ return function(){ let v0 = 1; return H + v0; } }",
        "function f(a, b){ var v1 = a; return function(c){ return v0 + c + b + v1; }; }",
        "let v0 = 1; function f(a){ return function(b){ return function(c){ return a + b + c + v2; }; }; }",
        "function f(v0, a){ function g(v1, b){ { let c = v3; } return a + b; } return g; }",
        "function f(a){ return (b) => (c) => { var v0, v1, v2; return a + b + c + v5 + v10; }; }",
        "function v1(a, b, c){ try { a() } catch (e) { var d = e; } for (let i = 0; i < b; i++) { c(i) } }",
        "class K { m(a){ return (b) => a + b + v0 } } function f(x, y){ return K + x + y }",
        "function f(a){ { let a = 2; { let b = a; } } return a; } function g(a){ return a }",
    ] {
        assert_matches_reference(code);
    }
}

#[test]
fn planner_matches_reference_on_generated_nesting() {
    // Deterministic pseudo-random programs: nested functions and blocks that
    // declare short names and slot-shaped names and read free ones.
    const NAMES: &[&str] = &["a", "b", "c", "v0", "v1", "v2", "v3", "v11", "x", "H"];
    let mut seed = 0x2545_f491_u64;
    let mut next = |bound: usize| {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        usize::try_from(seed >> 33).unwrap_or_default() % bound
    };
    for _ in 0..60 {
        let mut parts: Vec<String> = Vec::new();
        let mut open = 0;
        for _ in 0..(5 + next(25)) {
            match next(6) {
                0 | 1 => {
                    parts.push(format!("function f{}({}){{", next(100), NAMES[next(NAMES.len())]));
                    open += 1;
                }
                2 => {
                    parts.push("{".to_string());
                    open += 1;
                }
                3 => parts.push(format!("var {} = 1;", NAMES[next(NAMES.len())])),
                4 => parts.push(format!("g({});", NAMES[next(NAMES.len())])),
                _ if open > 0 => {
                    parts.push("}".to_string());
                    open -= 1;
                }
                _ => {}
            }
        }
        parts.push("}".repeat(open));
        assert_matches_reference(&parts.concat());
    }
}

#[test]
fn deep_nesting_needs_one_slot_search_per_candidate() {
    // `(a)=>(a(),` nested 3000 deep: every level picks a slot past all the
    // slots its ancestors took. The old planner probed each one in turn.
    let depth = 3000;
    let code = format!("var chain={}1{};", "(a)=>(a(),".repeat(depth), ")".repeat(depth));
    let (plan, stats) = crate::stack::run_on_sized_stack(code.len(), || {
        let alloc = Allocator::default();
        let parsed = Parser::new(&alloc, &code, SourceType::mjs()).parse();
        let scoping = semantic::scoping(&parsed.program);
        plan_local_renames_with_stats(
            &scoping,
            &FxHashSet::default(),
            RenameMode::Slot,
            &FxHashMap::default(),
            &FxHashMap::default(),
        )
    });
    assert_eq!(plan.len(), depth);
    assert_eq!(stats.slot_searches, depth);
    assert_eq!(stats.max_scope_depth, depth + 1);
    let innermost = plan
        .values()
        .filter(|name| name.as_str() == format!("v{}", depth - 1))
        .count();
    assert_eq!(innermost, 1, "each level takes the next slot");
}
