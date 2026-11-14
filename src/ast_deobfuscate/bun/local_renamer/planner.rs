use std::borrow::Cow;
use std::time::Instant;

use oxc_semantic::Scoping;
use oxc_syntax::scope::ScopeId;
use oxc_syntax::symbol::SymbolId;
use rustc_hash::{FxHashMap, FxHashSet};

use super::LocalRenameMap;
use super::names::{
    is_likely_generated_alias, is_meaningful_short, is_single_char_mangled, is_usable_base, pick_readable,
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

/// Like [`plan_local_renames`], and also returns counters for tracing.
///
/// A name `n` is blocked for a candidate in scope `S` when `n` is already
/// chosen in `S`, declared in `S` or a descendant, referenced free in `S` or a
/// descendant, or declared in or chosen for a proper ancestor of `S`. A
/// candidate may keep its own current name unless another candidate of `S`
/// took it.
///
/// The work is near-linear in scopes, symbols and free references, so deep
/// nesting in hostile input cannot make it explode:
/// - Scopes are visited once, in preorder, so a subtree is a contiguous
///   preorder range `[l, r]`. Each name keeps a sorted list of the preorder
///   positions where it is declared or referenced free, and a cursor to its
///   first position `>= l`; the cursor only moves forward because `l` does.
///   "Occurs in the subtree" is then `position <= r`.
/// - Names declared in or chosen for the scopes on the current root path are
///   counted per name, added on entering a subtree and removed on leaving it.
/// - `v{n}` slots are found with a max segment tree over `n` whose leaf is 0
///   while the name is blocked on the path and otherwise its next position. The
///   first free slot `>= counter` is the first leaf `> r`, found in
///   `O(log slots)` instead of probing slot after slot.
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
    let mut stats = LocalRenamePlanStats {
        symbols: scoping.symbols_len(),
        references: scoping.references_len(),
        scopes: scoping.scopes_len(),
        ..LocalRenamePlanStats::default()
    };

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

    let tree = ScopeTree::new(scoping);
    let scope_count = tree.len();

    for sym in scoping.symbol_ids() {
        stats.scope_name_entries += 1;
        if is_candidate(scoping.symbol_name(sym), sym) {
            stats.candidate_symbols += 1;
        }
    }
    stats.free_name_entries = free_names.values().map(FxHashSet::len).sum();
    // Every pick is below `slot_limit`: each slot below a pick is blocked, and
    // each blocked slot is a distinct name that is either an input name or a
    // name chosen earlier in this run.
    let slot_limit = stats.symbols + stats.free_name_entries + stats.candidate_symbols + 1;
    let mut names = NameTable::new(slot_limit);

    // Bindings and free names by preorder position.
    let mut declared: Vec<Vec<Binding<'_>>> = vec![Vec::new(); scope_count];
    for sym in scoping.symbol_ids() {
        let name = scoping.symbol_name(sym);
        let id = names.intern(Cow::Borrowed(name));
        declared[tree.position(scoping.symbol_scope_id(sym))].push(Binding { name, sym, id });
    }
    let mut free_at: Vec<Vec<usize>> = vec![Vec::new(); scope_count];
    for (&scope, scope_free) in free_names {
        let ids = scope_free.iter().map(|name| names.intern(Cow::Borrowed(name.as_str())));
        free_at[tree.position(scope)].extend(ids);
    }
    for pos in 0..scope_count {
        for id in declared[pos].iter().map(|b| b.id).chain(free_at[pos].iter().copied()) {
            names.add_position(id, pos);
        }
    }
    names.init_slots();

    let mut plan: LocalRenameMap = FxHashMap::default();
    // Open scopes on the current root path, innermost last.
    let mut path: Vec<OpenScope> = Vec::new();
    for pos in 0..scope_count {
        let last = tree.last[pos];
        while let Some(open) = path.pop_if(|open| open.last < pos) {
            for id in declared[open.pos].iter().map(|b| b.id).chain(open.assigned) {
                names.unblock(id);
            }
        }

        // Candidates in this scope, deterministic order for stable output.
        let mut cands: Vec<&Binding<'_>> = declared[pos].iter().filter(|b| is_candidate(b.name, b.sym)).collect();
        let mut local: FxHashSet<String> = FxHashSet::default();
        let mut assigned = Vec::new();
        if !cands.is_empty() {
            stats.candidate_scopes += 1;
            stats.max_scope_depth = stats.max_scope_depth.max(path.len() + 1);
            stats.max_candidates_in_scope = stats.max_candidates_in_scope.max(cands.len());
            cands.sort_by(|a, b| a.name.cmp(b.name).then_with(|| a.sym.index().cmp(&b.sym.index())));
        }
        let mut counter = 0usize;
        for &Binding {
            name: cur_name, sym, ..
        } in cands
        {
            // Readable/Sourcemap: try the proposed base (and base2…base9).
            // Readable/Slot then fall back to the next `v{n}` slot; Sourcemap
            // does NOT slot — if its name is fully blocked, the binding is left
            // unchanged (skipped).
            let proposed = (mode != RenameMode::Slot)
                .then(|| desired.get(&sym))
                .flatten()
                .filter(|b| is_usable_base(b))
                .and_then(|base| pick_readable(base, |c| names.is_blocked(c, cur_name, &local, last)));
            let cand = match (proposed, mode) {
                (Some(name), _) => Some(name),
                (None, RenameMode::Sourcemap) => None, // leave unchanged
                (None, _) => {
                    stats.slot_searches += 1;
                    // Never `None` (see `slot_limit`); leaving the binding
                    // unchanged would be safe anyway.
                    names.first_free_slot(counter, cur_name, &local, last).map(|slot| {
                        counter = slot + 1;
                        format!("v{slot}")
                    })
                }
            };
            if let Some(cand) = cand {
                let id = names.intern(Cow::Owned(cand.clone()));
                names.block(id);
                assigned.push(id);
                local.insert(cand.clone());
                plan.insert(sym, cand);
            } else {
                stats.skipped_symbols += 1;
            }
        }

        for binding in &declared[pos] {
            names.block(binding.id);
        }
        path.push(OpenScope { pos, last, assigned });
        // Every later scope starts after `pos`.
        for id in declared[pos].iter().map(|b| b.id).chain(free_at[pos].iter().copied()) {
            names.pass_position(id, pos);
        }
    }

    stats.planned_symbols = plan.len();
    debug_assert!(stats.candidate_scopes <= stats.scopes);
    debug_assert!(stats.planned_symbols <= stats.candidate_symbols);
    if trace {
        eprintln!(
            "[BUN_LOCAL_RENAME] total elapsed={:.3}s stats={stats:?}",
            started.elapsed().as_secs_f64()
        );
    }

    (plan, stats)
}

#[derive(Clone)]
struct Binding<'s> {
    name: &'s str,
    sym: SymbolId,
    id: usize,
}

/// A scope on the current root path and the names chosen for it.
struct OpenScope {
    pos: usize,
    last: usize,
    assigned: Vec<usize>,
}

/// The scope tree in preorder. A subtree is the position range
/// `[position(scope), last[position(scope)]]`.
struct ScopeTree {
    position_of: Vec<usize>,
    last: Vec<usize>,
}

impl ScopeTree {
    fn new(scoping: &Scoping) -> Self {
        let len = scoping.scopes_len();
        let mut children: Vec<Vec<ScopeId>> = vec![Vec::new(); len];
        let mut roots = Vec::new();
        for scope in scoping.scope_descendants_from_root() {
            match scoping.scope_parent_id(scope) {
                Some(parent) => children[parent.index()].push(scope),
                None => roots.push(scope),
            }
        }
        let mut position_of = vec![0; len];
        let mut last = vec![0; len];
        let mut visited = 0;
        // Explicit stack: nesting depth comes from the input.
        let mut stack: Vec<(ScopeId, usize)> = Vec::new();
        for root in roots {
            position_of[root.index()] = visited;
            visited += 1;
            stack.push((root, 0));
            while let Some((scope, next_child)) = stack.last_mut() {
                if let Some(&child) = children[scope.index()].get(*next_child) {
                    *next_child += 1;
                    position_of[child.index()] = visited;
                    visited += 1;
                    stack.push((child, 0));
                } else {
                    last[position_of[scope.index()]] = visited - 1;
                    stack.pop();
                }
            }
        }
        Self { position_of, last }
    }

    const fn len(&self) -> usize {
        self.last.len()
    }

    fn position(&self, scope: ScopeId) -> usize {
        self.position_of[scope.index()]
    }
}

/// Per-name blocking state, keyed by an interned id.
struct NameTable<'s> {
    ids: FxHashMap<Cow<'s, str>, usize>,
    /// Sorted preorder positions where the name is declared or referenced free.
    positions: Vec<Vec<usize>>,
    /// Index into `positions` of the first position not yet passed.
    cursor: Vec<usize>,
    /// Scopes on the root path that declare or chose the name, plus picks in
    /// the current scope.
    path_count: Vec<u32>,
    slot_of: Vec<Option<usize>>,
    slots: SlotTree,
}

impl<'s> NameTable<'s> {
    fn new(slot_limit: usize) -> Self {
        Self {
            ids: FxHashMap::default(),
            positions: Vec::new(),
            cursor: Vec::new(),
            path_count: Vec::new(),
            slot_of: Vec::new(),
            slots: SlotTree::new(slot_limit),
        }
    }

    fn id(&self, name: &str) -> Option<usize> {
        self.ids.get(name).copied()
    }

    fn intern(&mut self, name: Cow<'s, str>) -> usize {
        if let Some(&id) = self.ids.get(name.as_ref()) {
            return id;
        }
        let id = self.positions.len();
        let slot = slot_index(&name).filter(|&slot| slot < self.slots.len);
        self.ids.insert(name, id);
        self.positions.push(Vec::new());
        self.cursor.push(0);
        self.path_count.push(0);
        self.slot_of.push(slot);
        if let Some(slot) = slot {
            self.slots.set(slot, usize::MAX);
        }
        id
    }

    fn add_position(&mut self, id: usize, pos: usize) {
        let positions = &mut self.positions[id];
        if positions.last() != Some(&pos) {
            positions.push(pos);
        }
    }

    /// Load every slot name's first position into the slot tree.
    fn init_slots(&mut self) {
        for id in 0..self.positions.len() {
            self.refresh_slot(id);
        }
    }

    fn next_position(&self, id: usize) -> Option<usize> {
        self.positions[id].get(self.cursor[id]).copied()
    }

    fn refresh_slot(&mut self, id: usize) {
        if let Some(slot) = self.slot_of[id] {
            let value = if self.path_count[id] > 0 {
                0
            } else {
                self.next_position(id).unwrap_or(usize::MAX)
            };
            self.slots.set(slot, value);
        }
    }

    fn pass_position(&mut self, id: usize, pos: usize) {
        if self.next_position(id) == Some(pos) {
            self.cursor[id] += 1;
            self.refresh_slot(id);
        }
    }

    fn block(&mut self, id: usize) {
        self.path_count[id] += 1;
        if self.path_count[id] == 1 {
            self.refresh_slot(id);
        }
    }

    fn unblock(&mut self, id: usize) {
        self.path_count[id] -= 1;
        if self.path_count[id] == 0 {
            self.refresh_slot(id);
        }
    }

    /// Whether `name` is blocked for a candidate currently named `current_name`
    /// in the scope whose subtree ends at position `last`.
    fn is_blocked(&self, name: &str, current_name: &str, local: &FxHashSet<String>, last: usize) -> bool {
        if name == current_name {
            return local.contains(name);
        }
        local.contains(name)
            || self
                .id(name)
                .is_some_and(|id| self.path_count[id] > 0 || self.next_position(id).is_some_and(|pos| pos <= last))
    }

    /// The first slot `>= from` whose `v{n}` name is not blocked, or `None`
    /// when no slot below the tree's bound is free.
    fn first_free_slot(
        &self,
        from: usize,
        current_name: &str,
        local: &FxHashSet<String>,
        last: usize,
    ) -> Option<usize> {
        let free = self.slots.first_above(from, last)?;
        // The candidate's own name is free unless another candidate took it.
        Some(match slot_index(current_name) {
            Some(own) if (from..free).contains(&own) && !local.contains(current_name) => own,
            _ => free,
        })
    }
}

/// `n` when `name` is exactly the slot name `format!("v{n}")`.
fn slot_index(name: &str) -> Option<usize> {
    let digits = name.strip_prefix('v')?;
    let canonical = digits == "0" || (!digits.starts_with('0') && !digits.is_empty());
    if !canonical || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// Max segment tree over slot numbers. A leaf holds 0 while its name is
/// blocked on the root path, else the name's next preorder position
/// (`usize::MAX` for none, or for a slot name that never occurs).
struct SlotTree {
    len: usize,
    size: usize,
    max: Vec<usize>,
}

impl SlotTree {
    fn new(len: usize) -> Self {
        let size = len.next_power_of_two();
        Self {
            len,
            size,
            max: vec![usize::MAX; 2 * size],
        }
    }

    fn set(&mut self, slot: usize, value: usize) {
        let mut node = self.size + slot;
        self.max[node] = value;
        while node > 1 {
            node /= 2;
            self.max[node] = self.max[2 * node].max(self.max[2 * node + 1]);
        }
    }

    /// The first slot `>= from` and `< len` whose leaf is `> bound`.
    fn first_above(&self, from: usize, bound: usize) -> Option<usize> {
        self.find(1, 0, self.size, from, bound).filter(|&slot| slot < self.len)
    }

    fn find(&self, node: usize, lo: usize, hi: usize, from: usize, bound: usize) -> Option<usize> {
        if hi <= from || self.max[node] <= bound {
            return None;
        }
        if hi - lo == 1 {
            return Some(lo);
        }
        let mid = lo + (hi - lo) / 2;
        self.find(2 * node, lo, mid, from, bound)
            .or_else(|| self.find(2 * node + 1, mid, hi, from, bound))
    }
}
