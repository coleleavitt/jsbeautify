#[derive(Debug, Default, Clone, Copy)]
pub struct LocalRenamePlanStats {
    pub symbols: usize,
    pub references: usize,
    pub scopes: usize,
    pub scope_name_entries: usize,
    pub free_name_entries: usize,
    pub candidate_scopes: usize,
    pub candidate_symbols: usize,
    pub planned_symbols: usize,
    pub skipped_symbols: usize,
    /// Slot-name lookups; each costs `O(log slots)`.
    pub slot_searches: usize,
    pub max_scope_depth: usize,
    pub max_candidates_in_scope: usize,
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
