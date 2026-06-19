//! Sourcemap-driven original-name recovery.
//!
//! When a source map is available for a minified bundle, its mappings carry the
//! REAL identifier names — strictly better than any heuristic guess. This pass
//! resolves each binding's *position* to its original name and applies it
//! through the same scope-safe rename path as the heuristic renamer
//! ([`super::bun_local_renamer::RenameMode::Sourcemap`]).
//!
//! Crucially it is **span-keyed**, not name-keyed: a minified name like `a` maps
//! to dozens of distinct originals across scopes, so the flat
//! `minified→original` map (`cross_version::stable_names`) drops exactly the
//! short names we care about. Keying by `(line, column)` recovers each
//! occurrence independently.
//!
//! Because sourcemap positions refer to the ORIGINAL input, this runs as a
//! pre-step on the unmodified bundle (before the deobfuscate pipeline shifts
//! positions); residual minified names the map doesn't cover are then handled
//! by the heuristic pass.

use oxc_allocator::Allocator;
use oxc_ast::ast::BindingIdentifier;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_traverse::{ReusableTraverseCtx, Traverse, TraverseCtx, traverse_mut_with_ctx};
use rustc_hash::FxHashMap;
use std::hash::BuildHasher;

use crate::Result;
use crate::ast_deobfuscate::bun_local_renamer::{LocalRenameMap, LocalRenamer, RenameMode, plan_local_renames};
use crate::ast_deobfuscate::heuristic_rename::HeuristicNameCollector;
use crate::ast_deobfuscate::rename_guards::RenameGuardCollector;
use crate::ast_deobfuscate::state::DeobfuscateState;
use crate::cross_version::SourcemapParser;

type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

/// Byte offsets of each line start (`\n`+1), for offset→position conversion.
fn build_line_starts(source: &str) -> Vec<u32> {
    let mut starts = vec![0u32];
    for (i, ch) in source.char_indices() {
        if ch == '\n' {
            starts.push((i + 1) as u32);
        }
    }
    starts
}

/// Byte offset → (1-based line, 0-based UTF-16 column). MUST match the
/// sourcemap parser's `IdentifierCollector::offset_to_line_col` so binding
/// positions line up with the extracted name mappings.
fn offset_to_line_col(line_starts: &[u32], source: &str, offset: u32) -> (u32, u32) {
    let line_idx = line_starts.partition_point(|&start| start <= offset);
    let line = line_idx as u32;
    let line_start = line_starts.get(line_idx.saturating_sub(1)).copied().unwrap_or(0) as usize;
    let offset = offset as usize;
    let slice = &source.as_bytes()[line_start..offset.min(source.len())];
    let mut col = 0u32;
    let mut i = 0;
    while i < slice.len() {
        let b = slice[i];
        let (bytes, units) = if b < 0x80 {
            (1, 1)
        } else if b < 0xE0 {
            (2, 1)
        } else if b < 0xF0 {
            (3, 1)
        } else {
            (4, 2) // 4-byte UTF-8 = surrogate pair = 2 UTF-16 units
        };
        col += units;
        i += bytes;
    }
    (line, col)
}

/// Maps each binding's `SymbolId` to its sourcemap original name (by position).
struct SourcemapNameCollector<'c, S: BuildHasher> {
    by_pos: &'c std::collections::HashMap<(u32, u32), String, S>,
    line_starts: Vec<u32>,
    source: &'c str,
    desired: LocalRenameMap,
}

impl<'a, S: BuildHasher> Traverse<'a, DeobfuscateState> for SourcemapNameCollector<'_, S> {
    fn enter_binding_identifier(&mut self, it: &mut BindingIdentifier<'a>, _ctx: &mut Ctx<'a>) {
        let pos = offset_to_line_col(&self.line_starts, self.source, it.span.start);
        if let Some(orig) = self.by_pos.get(&pos)
            && orig.as_str() != it.name.as_str()
            && let Some(sym) = it.symbol_id.get()
        {
            self.desired.insert(sym, orig.clone());
        }
    }
}

/// Rename bindings to the original names given by `by_pos`.
///
/// The map is keyed by the binding's `(1-based line, 0-based UTF-16 col)`. Bindings
/// with no mapping are left untouched so the heuristic pass can name them later.
#[must_use]
pub fn apply_names_by_position<S: BuildHasher>(
    code: &str,
    by_pos: &std::collections::HashMap<(u32, u32), String, S>,
) -> String {
    if by_pos.is_empty() {
        return code.to_string();
    }
    let alloc = Allocator::default();
    let mut prog = Parser::new(&alloc, code, SourceType::mjs()).parse().program;

    // Soundness guards (eval / native exports).
    let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &alloc);
    let mut guard = RenameGuardCollector::new();
    traverse_mut_with_ctx(&mut guard, &mut prog, &mut ctx);
    drop(ctx);
    let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
    let guarded = guard.into_guarded(&scoping);

    // Free/global reference names (capture safety) — reuse the heuristic
    // collector's free-name tracking; its proposals are discarded.
    let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &alloc);
    let mut hc = HeuristicNameCollector::new();
    traverse_mut_with_ctx(&mut hc, &mut prog, &mut ctx);
    drop(ctx);
    let (_proposals, free_names) = hc.into_parts();

    // Resolve binding positions → original names.
    let line_starts = build_line_starts(code);
    let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &alloc);
    let mut collector = SourcemapNameCollector {
        by_pos,
        line_starts,
        source: code,
        desired: FxHashMap::default(),
    };
    traverse_mut_with_ctx(&mut collector, &mut prog, &mut ctx);
    drop(ctx);
    let desired = collector.desired;
    if desired.is_empty() {
        return code.to_string();
    }

    let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
    let plan = plan_local_renames(&scoping, &guarded, RenameMode::Sourcemap, &desired, &free_names);
    if plan.is_empty() {
        return code.to_string();
    }
    let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &alloc);
    let mut renamer = LocalRenamer::new(plan);
    traverse_mut_with_ctx(&mut renamer, &mut prog, &mut ctx);
    Codegen::new().build(&prog).code
}

/// Rename minified bindings to their sourcemap original names. `sourcemap_json`
/// is a standard source map (with `sourcesContent`). Returns the input
/// unchanged if the map yields no usable names.
///
/// # Errors
/// Returns an error only if the sourcemap JSON is malformed.
pub fn apply_sourcemap_names(code: &str, sourcemap_json: &str) -> Result<String> {
    let mappings = SourcemapParser::new().extract_names(sourcemap_json, code)?;
    let by_pos: FxHashMap<(u32, u32), String> = mappings
        .into_iter()
        .map(|m| ((m.minified_line, m.minified_column), m.original_name))
        .collect();
    Ok(apply_names_by_position(code, &by_pos))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renames_bindings_to_position_keyed_original_names() {
        // `function f(a,b){return a+b}` — a at col 11, b at col 13 (line 1).
        let code = "function f(a, b) {\n  return a + b;\n}";
        let mut by_pos = FxHashMap::default();
        by_pos.insert((1, 11), "left".to_string());
        by_pos.insert((1, 14), "right".to_string());
        let out = apply_names_by_position(code, &by_pos);
        assert!(out.contains("function f(left, right)"), "params: {out}");
        assert!(out.contains("return left + right"), "refs follow: {out}");
    }

    #[test]
    fn span_keyed_distinguishes_same_minified_name_across_scopes() {
        // Two functions each with a param `a`, mapped to DIFFERENT originals by
        // position — the flat name map would collapse these; span-keying must not.
        let code = "function f(a){return a}\nfunction g(a){return a}";
        let mut by_pos = FxHashMap::default();
        by_pos.insert((1, 11), "userId".to_string()); // f's a
        by_pos.insert((2, 11), "count".to_string()); // g's a
        let out = apply_names_by_position(code, &by_pos);
        assert!(
            out.contains("function f(userId)") && out.contains("return userId"),
            "{out}"
        );
        assert!(
            out.contains("function g(count)") && out.contains("return count"),
            "{out}"
        );
    }

    #[test]
    fn unmapped_bindings_are_left_for_the_heuristic_pass() {
        let code = "function f(a, b){ return a }";
        let mut by_pos = FxHashMap::default();
        by_pos.insert((1, 11), "value".to_string()); // only `a`
        let out = apply_names_by_position(code, &by_pos);
        assert!(out.contains("value"), "mapped binding renamed: {out}");
        assert!(
            out.contains(", b)") || out.contains(",b)"),
            "unmapped `b` must stay: {out}"
        );
    }
}
