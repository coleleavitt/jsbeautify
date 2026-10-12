use oxc_semantic::Scoping;
use oxc_syntax::scope::ScopeId;
use oxc_syntax::symbol::SymbolId;
use rustc_hash::{FxHashMap, FxHashSet};

use super::ExportNameMap;
use super::names::{is_reserved_word, is_valid_identifier, sanitize_identifier};

/// Make the collected rename map safe to apply.
///
/// Bun reuses the same minified name across non-overlapping scopes, and many
/// modules export the same public name (`ZodType`, `Client`, `Command`, …).
/// Applying the raw collected map can therefore (a) assign a JS reserved word
/// (`instanceof`) or (b) give two distinct symbols that share a scope the same
/// name — both produce invalid JavaScript (redeclaration / parse errors seen on
/// the real cli.js bundle). This finalizer guarantees every assigned name is a
/// legal identifier and unique along the relevant scope chain: existing
/// bindings in the declaration scope, ancestor bindings/assigned names, and
/// descendant bindings that could capture rewritten references. It suffixes
/// with `$N` on collision, and also whenever the name is in `reserved`.
#[must_use]
pub fn make_names_unique(map: ExportNameMap, scoping: &Scoping, reserved: &FxHashSet<&str>) -> ExportNameMap {
    if map.is_empty() {
        return map;
    }

    // Seed original names for bindings that are NOT being renamed away, so a
    // rename can't collide with a pre-existing identifier. The subtree index is
    // conservative: an ancestor rename must avoid descendant names because a
    // rewritten ancestor reference may be emitted inside that descendant scope.
    // Only the declaration scopes of renamed symbols are ever looked up in it.
    let renamed: FxHashSet<SymbolId> = map.keys().copied().collect();
    let targets: FxHashSet<ScopeId> = map.keys().map(|&sym| scoping.symbol_scope_id(sym)).collect();
    let mut original_by_scope: FxHashMap<ScopeId, FxHashSet<&str>> = FxHashMap::default();
    let mut original_subtree: FxHashMap<ScopeId, FxHashSet<&str>> = FxHashMap::default();
    for sym in scoping.symbol_ids() {
        if renamed.contains(&sym) {
            continue;
        }
        let scope = scoping.symbol_scope_id(sym);
        let name = scoping.symbol_name(sym);
        original_by_scope.entry(scope).or_default().insert(name);
        for visible in scoping.scope_ancestors(scope) {
            if targets.contains(&visible) {
                original_subtree.entry(visible).or_default().insert(name);
            }
        }
    }

    // Deterministic assignment order → stable output across runs / versions:
    // scope depth, then desired name, then the `Debug` text of the symbol id.
    let mut entries: Vec<(usize, String, SymbolId, String)> = map
        .into_iter()
        .map(|(sym, desired)| {
            let depth = scoping.scope_ancestors(scoping.symbol_scope_id(sym)).count();
            (depth, format!("{sym:?}"), sym, desired)
        })
        .collect();
    entries.sort_unstable_by(|a, b| a.0.cmp(&b.0).then_with(|| a.3.cmp(&b.3)).then_with(|| a.1.cmp(&b.1)));

    let mut out: ExportNameMap = FxHashMap::default();
    let mut assigned: FxHashMap<ScopeId, FxHashSet<String>> = FxHashMap::default();
    for (_, _, sym, desired) in entries {
        let scope = scoping.symbol_scope_id(sym);
        let safe_desired = if is_valid_identifier(&desired) {
            desired
        } else {
            sanitize_identifier(&desired)
        };
        let base = if is_reserved_word(&safe_desired) {
            format!("{safe_desired}$")
        } else {
            safe_desired
        };
        // Blocked: original names in the declaration subtree, and original or
        // assigned names of the declaration scope and its ancestors.
        let subtree = original_subtree.get(&scope);
        let ancestors: Vec<ScopeId> = scoping.scope_ancestors(scope).collect();
        let blocked = |name: &str| {
            subtree.is_some_and(|names| names.contains(name))
                || ancestors.iter().any(|anc| {
                    (*anc != scope && original_by_scope.get(anc).is_some_and(|names| names.contains(name)))
                        || assigned.get(anc).is_some_and(|names| names.contains(name))
                })
        };
        let mut candidate = base.clone();
        let mut n = 1u32;
        while blocked(&candidate) || reserved.contains(candidate.as_str()) {
            candidate = format!("{base}${n}");
            n += 1;
        }
        assigned.entry(scope).or_default().insert(candidate.clone());
        out.insert(sym, candidate);
    }
    out
}
