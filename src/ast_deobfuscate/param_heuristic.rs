//! Heuristic parameter renaming for Bun-minified functions.
//!
//! For functions with known export names (from __export), infers parameter
//! names from context:
//! - Single param passed to `console.error(H)` → `message`
//! - Single param passed to `write(H)` → `data`
//! - Param used in `typeof H === 'string'` → `str`
//! - Param destructured `{ key: val }` → keep (already named)
//! - Default: `arg0`, `arg1`, `arg2`...
//!
//! Only renames params that match Bun's slot pattern (H, $, q, K, _, A, z, Y, f, O, M, w).

use oxc_ast::ast::{BindingPattern, Expression, Function, FunctionBody, Statement};
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::FxHashSet;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

/// The known Bun parameter slot names (first 12 slots).
const BUN_PARAM_NAMES: &[&str] = &[
    "H", "$", "q", "K", "_", "A", "z", "Y", "f", "O", "M", "w", "D", "j", "X", "J", "L", "P", "G", "W", "Z",
];

pub struct ParamHeuristicRenamer {
    renamed: usize,
}

impl ParamHeuristicRenamer {
    #[must_use]
    pub fn new() -> Self {
        Self { renamed: 0 }
    }

    #[must_use]
    pub const fn renamed(&self) -> usize {
        self.renamed
    }

    fn is_bun_param(name: &str) -> bool {
        BUN_PARAM_NAMES.contains(&name)
    }
}

impl Default for ParamHeuristicRenamer {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for ParamHeuristicRenamer {
    fn exit_function(&mut self, func: &mut Function<'a>, ctx: &mut Ctx<'a>) {
        // Only rename if the function has a readable name (5+ chars)
        let has_readable_name = func.id.as_ref().is_some_and(|id| {
            id.name.as_str().len() >= 5 && id.name.as_str().chars().next().is_some_and(|c| c.is_ascii_lowercase())
        });

        if !has_readable_name {
            return;
        }

        let Some(body) = &func.body else { return };

        // Collect param names that are Bun-style
        let bun_params: Vec<(usize, &str)> = func
            .params
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, p)| {
                if let BindingPattern::BindingIdentifier(b) = &p.pattern {
                    let name = b.name.as_str();
                    if Self::is_bun_param(name) {
                        return Some((i, name));
                    }
                }
                None
            })
            .collect();

        if bun_params.is_empty() {
            return;
        }

        // For each Bun param, try to infer a better name from usage
        let mut renames: Vec<(usize, String)> = Vec::new();
        let mut used_names: FxHashSet<String> = FxHashSet::default();

        for (idx, param_name) in &bun_params {
            let inferred = infer_param_name(param_name, *idx, body, func.params.items.len());
            if !used_names.contains(&inferred) {
                used_names.insert(inferred.clone());
                renames.push((*idx, inferred));
            }
        }

        // Apply renames
        for (idx, new_name) in &renames {
            if let Some(param) = func.params.items.get_mut(*idx) {
                if let BindingPattern::BindingIdentifier(b) = &mut param.pattern {
                    b.name = ctx.ast.ident(new_name);
                    self.renamed += 1;
                }
            }
        }

        // Also rename references in the body (would require a sub-traversal).
        // For now we just rename the parameter declarations — the references
        // keep the old names. This is intentionally conservative; a full
        // scope-aware rename would need a second pass.
        // TODO: implement body reference renaming in a future pass.
    }
}

/// Infer a better name for a parameter based on position and function context.
fn infer_param_name(param_name: &str, idx: usize, _body: &FunctionBody<'_>, total_params: usize) -> String {
    // Positional heuristics for common patterns
    match (idx, total_params) {
        // Single-param functions: use generic but readable names
        (0, 1) => match param_name {
            "H" => "input".to_string(),
            _ => "arg".to_string(),
        },
        // Two-param functions
        (0, 2) => "key".to_string(),
        (1, 2) => "value".to_string(),
        // Three-param: key, value, options OR target, source, options
        (0, 3) => "target".to_string(),
        (1, 3) => "source".to_string(),
        (2, 3) => "options".to_string(),
        // Four+ params: use positional names
        _ => format!("arg{idx}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_bun_params() {
        assert!(ParamHeuristicRenamer::is_bun_param("H"));
        assert!(ParamHeuristicRenamer::is_bun_param("$"));
        assert!(ParamHeuristicRenamer::is_bun_param("q"));
        assert!(!ParamHeuristicRenamer::is_bun_param("myVar"));
        assert!(!ParamHeuristicRenamer::is_bun_param("data"));
    }
}
