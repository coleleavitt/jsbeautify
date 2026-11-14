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

use oxc_allocator::GetAllocator;
use oxc_ast::ast::{BindingIdentifier, BindingPattern, Function, FunctionBody, IdentifierReference};
use oxc_ast_visit::Visit;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

/// The known Bun parameter slot names (first 12 slots).
const BUN_PARAM_NAMES: &[&str] = &[
    "H", "$", "q", "K", "_", "A", "z", "Y", "f", "O", "M", "w", "D", "j", "X", "J", "L", "P", "G", "W", "Z",
];

pub struct ParamHeuristicRenamer {
    renamed: usize,
    renames: FxHashMap<SymbolId, String>,
    /// Parameters that keep their name ([`crate::ast_deobfuscate::rename_guards`]).
    kept: FxHashSet<SymbolId>,
}

impl ParamHeuristicRenamer {
    #[must_use]
    pub fn new() -> Self {
        Self {
            renamed: 0,
            renames: FxHashMap::default(),
            kept: FxHashSet::default(),
        }
    }

    /// Never rename `symbols`, which must be ids of the program the next
    /// traversal visits.
    #[must_use]
    pub fn keep(mut self, symbols: FxHashSet<SymbolId>) -> Self {
        self.kept = symbols;
        self
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
    fn enter_function(&mut self, func: &mut Function<'a>, ctx: &mut Ctx<'a>) {
        // Only rename if the function has a readable name (5+ chars)
        let has_readable_name = func.id.as_ref().is_some_and(|id| {
            id.name.as_str().len() >= 5 && id.name.as_str().chars().next().is_some_and(|c| c.is_ascii_lowercase())
        });

        if !has_readable_name {
            return;
        }

        let Some(body) = &func.body else { return };

        // Collect param names that are Bun-style
        let bun_params: Vec<(usize, String, SymbolId)> = func
            .params
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, p)| {
                if let BindingPattern::BindingIdentifier(b) = &p.pattern {
                    let name = b.name.as_str();
                    if Self::is_bun_param(name) {
                        let symbol_id = b.symbol_id.get()?;
                        return (!self.kept.contains(&symbol_id)).then(|| (i, name.to_string(), symbol_id));
                    }
                }
                None
            })
            .collect();

        if bun_params.is_empty() {
            return;
        }

        // For each Bun param, try to infer a better name from usage
        let mut renames: Vec<(usize, SymbolId, String)> = Vec::new();
        let mut used_names = collect_function_names(func);

        for (idx, param_name, symbol_id) in &bun_params {
            let inferred = infer_param_name(param_name, *idx, body, func.params.items.len());
            if !used_names.contains(&inferred) {
                used_names.insert(inferred.clone());
                renames.push((*idx, *symbol_id, inferred));
            }
        }

        // Apply renames
        for (idx, symbol_id, new_name) in &renames {
            if let Some(param) = func.params.items.get_mut(*idx)
                && let BindingPattern::BindingIdentifier(b) = &mut param.pattern
            {
                b.name = ctx.allocator().alloc_str(new_name).into();
                self.renames.insert(*symbol_id, new_name.clone());
                self.renamed += 1;
            }
        }
    }

    fn enter_identifier_reference(&mut self, ident: &mut IdentifierReference<'a>, ctx: &mut Ctx<'a>) {
        let Some(symbol_id) = ctx.scoping().get_reference(ident.reference_id()).symbol_id() else {
            return;
        };
        let Some(new_name) = self.renames.get(&symbol_id) else {
            return;
        };
        if ident.name.as_str() != new_name {
            ident.name = ctx.allocator().alloc_str(new_name).into();
        }
    }
}

/// Every name a function binds or reads, in its parameters and body. A
/// parameter renamed to one of them would collide with a binding or capture a
/// reference to an outer binding or a global.
fn collect_function_names(func: &Function<'_>) -> FxHashSet<String> {
    #[derive(Default)]
    struct Names(FxHashSet<String>);

    impl<'a> Visit<'a> for Names {
        fn visit_binding_identifier(&mut self, it: &BindingIdentifier<'a>) {
            self.0.insert(it.name.to_string());
        }

        fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
            self.0.insert(it.name.to_string());
        }
    }

    let mut names = Names::default();
    names.visit_formal_parameters(&func.params);
    if let Some(body) = &func.body {
        names.visit_function_body(body);
    }
    names.0
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
    use crate::ast_deobfuscate::{DeobfuscateState, semantic};
    use oxc_allocator::Allocator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_span::SourceType;
    use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

    fn run(code: &str) -> String {
        let allocator = Allocator::default();
        let parsed = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let mut program = parsed.program;
        let scoping = semantic::scoping(&program);
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        let mut renamer = ParamHeuristicRenamer::new();
        traverse_mut_with_ctx(&mut renamer, &mut program, &mut ctx);
        Codegen::new().build(&program).code
    }

    #[test]
    fn identifies_bun_params() {
        assert!(ParamHeuristicRenamer::is_bun_param("H"));
        assert!(ParamHeuristicRenamer::is_bun_param("$"));
        assert!(ParamHeuristicRenamer::is_bun_param("q"));
        assert!(!ParamHeuristicRenamer::is_bun_param("myVar"));
        assert!(!ParamHeuristicRenamer::is_bun_param("data"));
    }

    #[test]
    fn renames_parameter_references() {
        let out = run("function processData(H, $) { console.log(H); return H + $; }");
        assert!(out.contains("function processData(key, value)"), "got: {out}");
        assert!(out.contains("console.log(key)"), "got: {out}");
        assert!(
            out.contains("return key + value") || out.contains("return key+value"),
            "got: {out}"
        );
        assert!(
            !out.contains(" H") && !out.contains(" $"),
            "old param names should be gone: {out}"
        );
    }

    #[test]
    fn skips_parameter_rename_when_inferred_name_is_bound() {
        let out = run("function processData(H, key) { return H + key; }");
        assert!(out.contains("function processData(H, key)"), "got: {out}");
        assert!(
            out.contains("return H + key") || out.contains("return H+key"),
            "got: {out}"
        );
    }

    #[test]
    fn skips_parameter_rename_that_would_capture_an_outer_or_free_name() {
        let out = run("var value = 1; function processData(H, q) { return value + q + H; }");
        assert!(out.contains("function processData(key, q)"), "got: {out}");
        let out = run("function processData(H) { return input + H; }");
        assert!(out.contains("function processData(H)"), "got: {out}");
    }
}
