use oxc_ast::ast::{Expression, Statement};
use oxc_traverse::Traverse;
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::state::{DecoderInfo, DecoderType, DeobfuscateState, StringArrayInfo};

use super::Ctx;
use super::rotation::{apply_rotation, brute_force_rotation};
use super::types::{DecoderCandidate, RotationCandidate};

pub struct SelfDefendingStringArray {
    pub(super) wrappers: FxHashMap<String, Vec<String>>,
    pub(super) decoder_candidates: FxHashMap<String, DecoderCandidate>,
    pub(super) rotation_candidates: Vec<RotationCandidate>,
    /// All identifier names used as `const X = NAME;` aliases at top-level scope
    pub(super) top_level_aliases: FxHashMap<String, String>,
}
impl SelfDefendingStringArray {
    #[must_use]
    pub fn new() -> Self {
        Self {
            wrappers: FxHashMap::default(),
            decoder_candidates: FxHashMap::default(),
            rotation_candidates: Vec::new(),
            top_level_aliases: FxHashMap::default(),
        }
    }

    pub fn finalize(&mut self, state: &mut DeobfuscateState) {
        // Resolve decoder candidates: keep only those whose wrapper_ref is a known wrapper
        let valid_decoders: FxHashMap<String, DecoderCandidate> = self
            .decoder_candidates
            .iter()
            .filter(|(_, c)| self.wrappers.contains_key(&c.wrapper_ref))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        // Resolve rotation candidates
        for rot in &self.rotation_candidates {
            if !self.wrappers.contains_key(&rot.wrapper_ref) {
                continue;
            }
            let Some(decoder) = valid_decoders.get(&rot.decoder_ref) else {
                continue;
            };

            let Some(strings) = self.wrappers.get_mut(&rot.wrapper_ref) else {
                continue;
            };

            let rotation_count =
                brute_force_rotation(strings, &rot.terms, rot.target, decoder.offset, decoder.offset_op);

            match rotation_count {
                Some(count) => {
                    apply_rotation(strings, count);
                    deob_trace!(
                        "[SELF-DEFENDING] Applied checksum rotation to {}: {} rotations (target={})",
                        rot.wrapper_ref,
                        count,
                        rot.target
                    );
                }
                None => {
                    eprintln!(
                        "[SELF-DEFENDING] WARNING: Could not solve checksum for {} (target={})",
                        rot.wrapper_ref, rot.target
                    );
                }
            }
        }

        for (name, strings) in &self.wrappers {
            let rotated = self.rotation_candidates.iter().any(|r| r.wrapper_ref == *name);
            state.string_arrays.insert(
                name.clone(),
                StringArrayInfo {
                    var_name: name.clone(),
                    strings: strings.clone(),
                    rotated,
                    rotation_count: 0,
                },
            );
        }

        for (dec_name, candidate) in &valid_decoders {
            state.decoders.insert(
                dec_name.clone(),
                DecoderInfo {
                    function_name: dec_name.clone(),
                    array_name: candidate.wrapper_ref.clone(),
                    offset: candidate.offset,
                    offset_operation: candidate.offset_op,
                    decoder_type: DecoderType::Simple,
                },
            );
        }

        // Also register aliases as decoders (e.g. `const _0x126786 = _0x5eba;`)
        for (alias, target) in &self.top_level_aliases {
            if let Some(dec) = valid_decoders.get(target)
                && !state.decoders.contains_key(alias)
            {
                state.decoders.insert(
                    alias.clone(),
                    DecoderInfo {
                        function_name: alias.clone(),
                        array_name: dec.wrapper_ref.clone(),
                        offset: dec.offset,
                        offset_operation: dec.offset_op,
                        decoder_type: DecoderType::Simple,
                    },
                );
            }
        }
    }
}

impl Default for SelfDefendingStringArray {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for SelfDefendingStringArray {
    fn enter_statement(&mut self, stmt: &mut Statement<'a>, _ctx: &mut Ctx<'a>) {
        if let Statement::FunctionDeclaration(func_decl) = stmt {
            let func_name = func_decl.id.as_ref().map_or("", |id| id.name.as_str());
            if !func_name.is_empty() {
                self.detect_wrapper(func_decl, func_name);
                self.detect_decoder_candidate(func_decl, func_name);
            }
        }

        if let Statement::ExpressionStatement(expr_stmt) = stmt {
            match &expr_stmt.expression {
                Expression::CallExpression(call) => {
                    self.detect_rotation_candidate(call);
                }
                Expression::ParenthesizedExpression(paren) => {
                    if let Expression::CallExpression(call) = &paren.expression {
                        self.detect_rotation_candidate(call);
                    }
                }
                _ => {}
            }
        }

        self.detect_alias(stmt);
    }
}
