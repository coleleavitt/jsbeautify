use oxc_semantic::Scoping;
use oxc_syntax::scope::ScopeId;
use oxc_syntax::symbol::SymbolId;
use rustc_hash::{FxHashMap, FxHashSet};
use std::time::Instant;

use super::LocalRenameMap;
use super::names::{
    is_blocked_name, is_likely_generated_alias, is_meaningful_short, is_single_char_mangled, is_usable_base,
    pick_readable,
};
use super::types::{LocalRenamePlanStats, RenameMode};

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
// The Fx* hashers are deliberate (perf + determinism); generalizing the public
// signature over arbitrary hashers would force every caller to be generic too.
#[allow(clippy::implicit_hasher)]
#[must_use]
pub fn plan_local_renames(
    scoping: &Scoping,
    exclude: &FxHashSet<SymbolId>,
    mode: RenameMode,
    desired: &LocalRenameMap,
    free_names: &FxHashMap<ScopeId, FxHashSet<String>>,
) -> LocalRenameMap {
    plan_local_renames_with_stats(scoping, exclude, mode, desired, free_names).0
}

#[allow(clippy::implicit_hasher, clippy::too_many_lines)]
#[must_use]
pub fn plan_local_renames_with_stats(
    scoping: &Scoping,
    exclude: &FxHashSet<SymbolId>,
    mode: RenameMode,
    desired: &LocalRenameMap,
    free_names: &FxHashMap<ScopeId, FxHashSet<String>>,
) -> (LocalRenameMap, LocalRenamePlanStats) {
    let trace = crate::ast_deobfuscate::deob_trace_enabled();
    let started = Instant::now();
    let mut step_started = started;
    let mut stats = LocalRenamePlanStats {
        symbols: scoping.symbols_len(),
        references: scoping.references_len(),
        scopes: scoping.scopes_len(),
        ..LocalRenamePlanStats::default()
    };

    // Original binding names per scope (from every symbol's declaring scope).
    let mut scope_names: FxHashMap<ScopeId, Vec<(&str, SymbolId)>> = FxHashMap::default();
    let mut scope_name_sets: FxHashMap<ScopeId, FxHashSet<&str>> = FxHashMap::default();
    let mut subtree_names: FxHashMap<ScopeId, FxHashSet<&str>> = FxHashMap::default();
    for sym in scoping.symbol_ids() {
        let scope = scoping.symbol_scope_id(sym);
        let name = scoping.symbol_name(sym);
        scope_names.entry(scope).or_default().push((name, sym));
        scope_name_sets.entry(scope).or_default().insert(name);
        stats.scope_name_entries += 1;
        for visible in scoping.scope_ancestors(scope) {
            stats.subtree_ancestor_visits += 1;
            if subtree_names.entry(visible).or_default().insert(name) {
                stats.subtree_name_entries += 1;
            }
        }
    }
    if trace {
        eprintln!(
            "[BUN_LOCAL_RENAME] scope/subtree name maps elapsed={:.3}s symbols={} scopes={} ancestor_visits={} subtree_entries={}",
            step_started.elapsed().as_secs_f64(),
            stats.symbols,
            stats.scopes,
            stats.subtree_ancestor_visits,
            stats.subtree_name_entries
        );
        step_started = Instant::now();
    }

    // Free (unresolved) reference names per scope SUBTREE: propagate each free
    // name up to all ancestor scopes (a binding is visible to its descendants,
    // so a rename in scope S can capture a free reference anywhere under S).
    let mut free_subtree: FxHashMap<ScopeId, FxHashSet<&str>> = FxHashMap::default();
    for (&scope, names) in free_names {
        stats.free_name_entries += names.len();
        for anc in scoping.scope_ancestors(scope) {
            let bucket = free_subtree.entry(anc).or_default();
            for n in names {
                stats.free_ancestor_visits += 1;
                if bucket.insert(n.as_str()) {
                    stats.free_subtree_entries += 1;
                }
            }
        }
    }
    if trace {
        eprintln!(
            "[BUN_LOCAL_RENAME] free-name propagation elapsed={:.3}s free_names={} ancestor_visits={} free_subtree_entries={}",
            step_started.elapsed().as_secs_f64(),
            stats.free_name_entries,
            stats.free_ancestor_visits,
            stats.free_subtree_entries
        );
        step_started = Instant::now();
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
    stats.candidate_scopes = scopes.len();
    stats.max_scope_depth = scopes.iter().map(|(depth, _)| *depth).max().unwrap_or_default();
    scopes.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.index().cmp(&b.1.index())));
    if trace {
        eprintln!(
            "[BUN_LOCAL_RENAME] candidate scope discovery elapsed={:.3}s candidate_scopes={} max_depth={}",
            step_started.elapsed().as_secs_f64(),
            stats.candidate_scopes,
            stats.max_scope_depth
        );
        step_started = Instant::now();
    }

    let mut plan: LocalRenameMap = FxHashMap::default();
    let mut assigned: FxHashMap<ScopeId, FxHashSet<String>> = FxHashMap::default();

    for (_depth, scope) in scopes {
        // Names that must not be reused in this scope. Descendant bindings are
        // checked because references to this symbol may be emitted inside those
        // scopes after renaming; choosing a descendant's existing name would be
        // captured by that descendant binding at runtime. Keep the checks lazy:
        // materializing this union for every candidate scope is the hot path on
        // large Bun bundles.
        let subtree_blocked = subtree_names.get(&scope);
        let free_blocked = free_subtree.get(&scope);
        stats.max_subtree_blocked = stats.max_subtree_blocked.max(subtree_blocked.map_or(0, FxHashSet::len));
        stats.max_free_blocked = stats.max_free_blocked.max(free_blocked.map_or(0, FxHashSet::len));
        let ancestor_scopes: Vec<ScopeId> = scoping.scope_ancestors(scope).skip(1).collect();
        let ancestor_blocked_upper_bound: usize = ancestor_scopes
            .iter()
            .map(|anc| scope_name_sets.get(anc).map_or(0, FxHashSet::len) + assigned.get(anc).map_or(0, FxHashSet::len))
            .sum();
        stats.max_blocked_names = stats.max_blocked_names.max(
            subtree_blocked.map_or(0, FxHashSet::len)
                + free_blocked.map_or(0, FxHashSet::len)
                + ancestor_blocked_upper_bound,
        );

        // Candidates in this scope, deterministic order for stable output.
        let mut cands: Vec<(&str, SymbolId)> = scope_names
            .get(&scope)
            .map_or_default(|names| names.iter().filter(|(n, s)| is_candidate(n, *s)).copied().collect());
        stats.candidate_symbols += cands.len();
        stats.max_candidates_in_scope = stats.max_candidates_in_scope.max(cands.len());
        cands.sort_by(|a, b| a.0.cmp(b.0).then_with(|| a.1.index().cmp(&b.1.index())));

        let mut local: FxHashSet<String> = FxHashSet::default();
        let mut counter = 0usize;
        for (cur_name, sym) in cands {
            // Readable/Sourcemap: try the proposed base (and base2…base9).
            // Readable/Slot then fall back to the next `v{n}` slot; Sourcemap
            // does NOT slot — if its name is fully blocked, the binding is left
            // unchanged (skipped).
            let proposed = (mode != RenameMode::Slot)
                .then(|| desired.get(&sym))
                .flatten()
                .filter(|b| is_usable_base(b))
                .and_then(|base| {
                    pick_readable(base, |candidate| {
                        is_blocked_name(
                            candidate,
                            cur_name,
                            &local,
                            subtree_blocked,
                            free_blocked,
                            &ancestor_scopes,
                            &scope_name_sets,
                            &assigned,
                        )
                    })
                });
            let cand = match (proposed, mode) {
                (Some(name), _) => Some(name),
                (None, RenameMode::Sourcemap) => None, // leave unchanged
                (None, _) => Some(loop {
                    let c = format!("v{counter}");
                    counter += 1;
                    if !is_blocked_name(
                        &c,
                        cur_name,
                        &local,
                        subtree_blocked,
                        free_blocked,
                        &ancestor_scopes,
                        &scope_name_sets,
                        &assigned,
                    ) {
                        break c;
                    }
                }),
            };
            if let Some(cand) = cand {
                local.insert(cand.clone());
                plan.insert(sym, cand);
            } else {
                stats.skipped_symbols += 1;
            }
        }
        if !local.is_empty() {
            assigned.insert(scope, local);
        }
    }

    stats.planned_symbols = plan.len();
    debug_assert!(stats.candidate_scopes <= stats.scopes);
    debug_assert!(stats.planned_symbols <= stats.candidate_symbols);
    if trace {
        eprintln!(
            "[BUN_LOCAL_RENAME] planning loop elapsed={:.3}s candidates={} planned={} max_blocked={} max_candidates_in_scope={}",
            step_started.elapsed().as_secs_f64(),
            stats.candidate_symbols,
            stats.planned_symbols,
            stats.max_blocked_names,
            stats.max_candidates_in_scope
        );
        eprintln!(
            "[BUN_LOCAL_RENAME] total elapsed={:.3}s stats={stats:?}",
            started.elapsed().as_secs_f64()
        );
    }

    (plan, stats)
}
