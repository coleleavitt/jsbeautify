//! Slot-based variable normalizer for Bun-minified bundles
//!
//! Converts minified variable names to canonical slot-based names (s0, s1, s2, ...)
//! that are stable across different Bun versions regardless of alphabet ordering.

use oxc_ast::ast::{BindingIdentifier, Function, Ident, IdentifierReference};
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::bun_alphabet::{BunAlphabet, extract_alphabet_from_source};
use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

mod identifiers;
mod names;

use identifiers::extract_identifiers;
use names::{GLOBALS, JS_KEYWORDS};

pub struct DeterministicRenamer {
    alphabet: BunAlphabet,
    rename_map: FxHashMap<String, String>,
    skip_names: FxHashSet<&'static str>,
    changed: bool,
    renamed_count: usize,
}

impl DeterministicRenamer {
    #[must_use]
    pub fn new() -> Self {
        let mut skip_names = FxHashSet::default();
        for &kw in JS_KEYWORDS {
            skip_names.insert(kw);
        }
        for &g in GLOBALS {
            skip_names.insert(g);
        }

        Self {
            alphabet: BunAlphabet::default_alphabet(),
            rename_map: FxHashMap::default(),
            skip_names,
            changed: false,
            renamed_count: 0,
        }
    }

    #[must_use]
    pub const fn has_changed(&self) -> bool {
        self.changed
    }

    #[must_use]
    pub const fn renamed_count(&self) -> usize {
        self.renamed_count
    }

    /// Extracts the alphabet from the source and builds the rename map
    pub fn analyze_source(&mut self, source: &str) {
        self.alphabet = extract_alphabet_from_source(source);
        eprintln!(
            "[SLOT_RENAME] Extracted HEAD: {}",
            &self.alphabet.head[..20.min(self.alphabet.head.len())]
        );
        eprintln!(
            "[SLOT_RENAME] Extracted TAIL: {}",
            &self.alphabet.tail[..20.min(self.alphabet.tail.len())]
        );
    }

    /// Builds the rename map from minified names to slot-based names
    pub fn build_rename_map(&mut self, source: &str) {
        let mut identifier_slots: Vec<(String, usize)> = Vec::new();

        for name in extract_identifiers(source) {
            if self.should_skip(&name) {
                continue;
            }

            if let Some(slot) = self.alphabet.name_to_slot(&name)
                && !self.rename_map.contains_key(&name)
            {
                identifier_slots.push((name, slot));
            }
        }

        for (name, slot) in identifier_slots {
            let canonical = format!("s{slot}");
            self.rename_map.insert(name, canonical);
        }

        eprintln!("[SLOT_RENAME] Built rename map with {} entries", self.rename_map.len());
    }

    fn should_skip(&self, name: &str) -> bool {
        if self.skip_names.contains(name) {
            return true;
        }

        if name.starts_with("_0x") {
            return true;
        }

        if name.len() > 4 {
            return true;
        }

        if name.len() > 2 && name.starts_with('_') && !name.chars().nth(1).is_some_and(|c| c.is_ascii_digit()) {
            return true;
        }

        false
    }

    fn try_rename(&mut self, name: &str) -> Option<&str> {
        self.rename_map.get(name).map(String::as_str)
    }
}

impl Default for DeterministicRenamer {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for DeterministicRenamer {
    fn enter_binding_identifier(&mut self, ident: &mut BindingIdentifier<'a>, ctx: &mut Ctx<'a>) {
        let old_name = ident.name.as_str();
        if let Some(new_name) = self.try_rename(old_name) {
            ident.name = Ident::from_str_in(new_name, ctx);
            self.changed = true;
            self.renamed_count += 1;
        }
    }

    fn enter_identifier_reference(&mut self, ident: &mut IdentifierReference<'a>, ctx: &mut Ctx<'a>) {
        let old_name = ident.name.as_str();
        if let Some(new_name) = self.try_rename(old_name) {
            ident.name = Ident::from_str_in(new_name, ctx);
            self.changed = true;
            self.renamed_count += 1;
        }
    }

    fn enter_function(&mut self, func: &mut Function<'a>, ctx: &mut Ctx<'a>) {
        if let Some(ident) = &mut func.id {
            let old_name = ident.name.as_str();
            if let Some(new_name) = self.try_rename(old_name) {
                ident.name = Ident::from_str_in(new_name, ctx);
                self.changed = true;
                self.renamed_count += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests;
