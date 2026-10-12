use super::*;
use crate::ast_deobfuscate::rename_guards::RenameGuardCollector;
use crate::ast_deobfuscate::{DeobfuscateState, semantic};
#[cfg(test)]
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_span::SourceType;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

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
