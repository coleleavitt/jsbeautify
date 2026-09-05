//! Canonical naming for cross-version alignment.

use oxc_ast::ast::{BindingIdentifier, IdentifierReference};
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::ast_matcher::{StatementInfo, StatementMatcher};

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct CanonicalNamer {
    stable_names: FxHashMap<String, String>,
    canonical_names: FxHashMap<String, String>,
    canonical_counter: usize,
    position_to_canonical: FxHashMap<(u32, u32), String>,
    changed: bool,
    renamed_count: usize,
}

impl CanonicalNamer {
    #[must_use]
    pub fn new() -> Self {
        Self {
            stable_names: FxHashMap::default(),
            canonical_names: FxHashMap::default(),
            canonical_counter: 0,
            position_to_canonical: FxHashMap::default(),
            changed: false,
            renamed_count: 0,
        }
    }

    pub fn load_stable_names(&mut self, names: FxHashMap<String, String>) {
        self.stable_names = names;
    }

    pub fn build_canonical_map(&mut self, source_code: &str, target_code: &str, matcher: &StatementMatcher) {
        let source_stmts = matcher.extract_statements(source_code);
        let target_stmts = matcher.extract_statements(target_code);
        let matched_pairs = matcher.match_statements(&source_stmts, &target_stmts);

        for (source_stmt, target_stmt) in matched_pairs {
            self.process_matched_pair(source_stmt, target_stmt);
        }
    }

    fn process_matched_pair(&mut self, source_stmt: &StatementInfo, target_stmt: &StatementInfo) {
        let mut order_to_canonical: FxHashMap<usize, String> = FxHashMap::default();

        for src_id in &source_stmt.identifiers {
            let canonical = if let Some(stable) = self.stable_names.get(&src_id.name) {
                stable.clone()
            } else {
                let key = format!("{}:{}", source_stmt.hash, src_id.order_index);
                if !self.canonical_names.contains_key(&key) {
                    let name = format!("_r{}", self.canonical_counter);
                    self.canonical_counter += 1;
                    self.canonical_names.insert(key.clone(), name);
                }
                self.canonical_names
                    .get(&key)
                    .expect("canonical name exists for key")
                    .clone()
            };

            order_to_canonical.insert(src_id.order_index, canonical.clone());
            self.position_to_canonical.insert((src_id.start, src_id.end), canonical);
        }

        for tgt_id in &target_stmt.identifiers {
            if let Some(canonical) = order_to_canonical.get(&tgt_id.order_index) {
                self.position_to_canonical
                    .insert((tgt_id.start, tgt_id.end), canonical.clone());
            }
        }
    }

    fn get_canonical_at(&self, start: u32, end: u32) -> Option<&str> {
        self.position_to_canonical.get(&(start, end)).map(String::as_str)
    }

    #[must_use]
    pub const fn has_changed(&self) -> bool {
        self.changed
    }

    #[must_use]
    pub const fn renamed_count(&self) -> usize {
        self.renamed_count
    }

    #[must_use]
    pub const fn canonical_names_generated(&self) -> usize {
        self.canonical_counter
    }
}

impl Default for CanonicalNamer {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for CanonicalNamer {
    fn enter_binding_identifier(&mut self, ident: &mut BindingIdentifier<'a>, ctx: &mut Ctx<'a>) {
        if let Some(canonical) = self.get_canonical_at(ident.span.start, ident.span.end)
            && canonical != ident.name.as_str()
        {
            ident.name = oxc_ast::ast::Ident::from_str_in(canonical, ctx);
            self.changed = true;
            self.renamed_count += 1;
        }
    }

    fn enter_identifier_reference(&mut self, ident: &mut IdentifierReference<'a>, ctx: &mut Ctx<'a>) {
        if let Some(canonical) = self.get_canonical_at(ident.span.start, ident.span.end)
            && canonical != ident.name.as_str()
        {
            ident.name = oxc_ast::ast::Ident::from_str_in(canonical, ctx);
            self.changed = true;
            self.renamed_count += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_canonical_namer_creation() {
        let namer = CanonicalNamer::new();
        assert_eq!(namer.canonical_names_generated(), 0);
    }

    #[test]
    fn test_stable_name_loading() {
        let mut namer = CanonicalNamer::new();
        let mut stable = FxHashMap::default();
        stable.insert("a".to_string(), "originalA".to_string());
        namer.load_stable_names(stable);
        assert_eq!(namer.stable_names.len(), 1);
    }
}
