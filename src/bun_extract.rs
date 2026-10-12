//! Bun-specific name extraction and module splitting.
//!
//! Extracts original names from Bun bundle patterns:
//! - `MR()` export mappings: `MR(target, { exportName: () => minifiedVar })`
//! - this.name patterns: `this.name = "ClassName"` in class constructors
//! - displayName patterns: `X.displayName = "ComponentName"`
//! - Module wrappers: calls to the detected `__commonJS` and `__esm` helpers

use oxc_ast::ast::{
    AssignmentExpression, AssignmentOperator, AssignmentTarget, BindingPattern, CallExpression, Expression,
    VariableDeclaration,
};
use oxc_ast_visit::{Visit, walk};
use oxc_semantic::Scoping;
use oxc_span::GetSpan;
use oxc_syntax::symbol::SymbolId;
use regex::Regex;
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::bun::{BunHelper, Bundler, with_bundle_runtime};
use std::sync::LazyLock;

/// Extracted name mapping from `MR()` export calls
#[derive(Debug, Clone)]
pub struct ExportMapping {
    pub minified_name: String,
    pub original_name: String,
    pub source: MappingSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MappingSource {
    MrExport,
    ThisName,
    DisplayName,
}

/// Module info extracted from bundle
#[derive(Debug, Clone)]
pub struct ModuleInfo {
    pub name: String,
    pub module_type: ModuleType,
    pub start: usize,
    pub end: usize,
    pub body_start: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleType {
    CommonJs,
    Esm,
    Runtime,
    Main,
}

static MR_CALL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:MR|__export|j\$)\([^,]+,\s*\{").expect("valid regex pattern"));

static EXPORT_PAIR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"([A-Za-z_$][A-Za-z0-9_$]*):\s*\(\)\s*=>\s*([A-Za-z_$][A-Za-z0-9_$]*)").expect("valid regex pattern")
});

static CLASS_DECL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"class\s+(\w+)\s+extends\s+\w+[^{]*\{").expect("valid regex pattern"));

static CLASS_ASSIGN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\w+)\s*=\s*class\s+(?:\w+\s+)?extends\s+\w+[^{]*\{").expect("valid regex pattern"));

static THIS_NAME_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"this\.name\s*=\s*"([^"]+)""#).expect("valid regex pattern"));

static DISPLAY_NAME_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(\w+)\.displayName\s*=\s*"([^"]+)""#).expect("valid regex pattern"));

const GENERIC_NAMES: &[&str] = &[
    "call", "default", "get", "set", "init", "run", "exec", "then", "next", "done",
];

/// Extract all `MR()` export mappings from code.
/// Returns mappings from minified variable name to original export name.
///
/// # Panics
/// Panics only if a hard-coded regex capture group is missing after a successful
/// match, which would indicate an internal regex definition bug.
#[must_use]
pub fn extract_mr_exports(code: &str) -> Vec<ExportMapping> {
    let mut results = Vec::new();

    for m in MR_CALL_RE.find_iter(code) {
        let start_idx = m.end();

        // Find matching closing brace
        let mut depth = 1;
        let mut end_idx = start_idx;
        let bytes = code.as_bytes();

        while end_idx < bytes.len() && depth > 0 {
            match bytes[end_idx] {
                b'{' => depth += 1,
                b'}' => depth -= 1,
                _ => {}
            }
            end_idx += 1;
        }

        if depth != 0 {
            continue;
        }

        let body = &code[start_idx..end_idx - 1];

        for cap in EXPORT_PAIR_RE.captures_iter(body) {
            let export_name = cap.get(1).expect("capture group 1 guaranteed by regex").as_str();
            let minified_var = cap.get(2).expect("capture group 2 guaranteed by regex").as_str();

            // Skip if same name or generic
            if export_name == minified_var {
                continue;
            }
            if GENERIC_NAMES.contains(&export_name) {
                continue;
            }
            if export_name.len() < 3 {
                continue;
            }
            if export_name.starts_with('_') {
                continue;
            }

            results.push(ExportMapping {
                minified_name: minified_var.to_string(),
                original_name: export_name.to_string(),
                source: MappingSource::MrExport,
            });
        }
    }

    results
}

/// Extract class names from `this.name = "ClassName"` patterns.
///
/// # Panics
/// Panics only if a hard-coded regex capture group is missing after a successful
/// match, which would indicate an internal regex definition bug.
#[must_use]
pub fn extract_this_name_patterns(code: &str) -> Vec<ExportMapping> {
    let mut results = Vec::new();

    // Pattern 1: class X extends Y { ... this.name = "Z" }
    for cap in CLASS_DECL_RE.captures_iter(code) {
        let class_name = cap.get(1).expect("capture group 1 guaranteed by regex").as_str();
        let start_idx = cap.get(0).expect("capture group 0 guaranteed by regex").end();

        // Look within next 500 chars for this.name = "..."
        let end = (start_idx + 500).min(code.len());
        let window = &code[start_idx..end];

        if let Some(name_cap) = THIS_NAME_RE.captures(window) {
            let readable_name = name_cap.get(1).expect("capture group 1 guaranteed by regex").as_str();
            if class_name != readable_name && class_name.len() > 1 {
                results.push(ExportMapping {
                    minified_name: class_name.to_string(),
                    original_name: readable_name.to_string(),
                    source: MappingSource::ThisName,
                });
            }
        }
    }

    // Pattern 2: X = class extends Y { ... this.name = "Z" }
    for cap in CLASS_ASSIGN_RE.captures_iter(code) {
        let var_name = cap.get(1).expect("capture group 1 guaranteed by regex").as_str();
        let start_idx = cap.get(0).expect("capture group 0 guaranteed by regex").end();

        let end = (start_idx + 500).min(code.len());
        let window = &code[start_idx..end];

        if let Some(name_cap) = THIS_NAME_RE.captures(window) {
            let readable_name = name_cap.get(1).expect("capture group 1 guaranteed by regex").as_str();
            if var_name != readable_name && var_name.len() > 1 {
                // Avoid duplicates
                let exists = results
                    .iter()
                    .any(|r| r.minified_name == var_name && r.original_name == readable_name);
                if !exists {
                    results.push(ExportMapping {
                        minified_name: var_name.to_string(),
                        original_name: readable_name.to_string(),
                        source: MappingSource::ThisName,
                    });
                }
            }
        }
    }

    results
}

/// Extract displayName assignments: `X.displayName = "ComponentName"`
///
/// # Panics
/// Panics only if a hard-coded regex capture group is missing after a successful
/// match, which would indicate an internal regex definition bug.
#[must_use]
pub fn extract_display_name_patterns(code: &str) -> Vec<ExportMapping> {
    let mut results = Vec::new();

    for cap in DISPLAY_NAME_RE.captures_iter(code) {
        let minified_var = cap.get(1).expect("capture group 1 guaranteed by regex").as_str();
        let readable_name = cap.get(2).expect("capture group 2 guaranteed by regex").as_str();

        if minified_var != readable_name && minified_var.len() > 1 && !readable_name.starts_with('#') {
            results.push(ExportMapping {
                minified_name: minified_var.to_string(),
                original_name: readable_name.to_string(),
                source: MappingSource::DisplayName,
            });
        }
    }

    results
}

/// Extract all name mappings from code (MR exports + this.name + displayName).
/// Returns deduplicated map from minified name to original name.
#[must_use]
pub fn extract_all_names(code: &str) -> FxHashMap<String, String> {
    let mut var_map: FxHashMap<String, Vec<String>> = FxHashMap::default();

    let mr_exports = extract_mr_exports(code);
    let this_names = extract_this_name_patterns(code);
    let display_names = extract_display_name_patterns(code);

    for mapping in mr_exports.into_iter().chain(this_names).chain(display_names) {
        var_map
            .entry(mapping.minified_name)
            .or_default()
            .push(mapping.original_name);
    }

    // Build final map, only include non-conflicting mappings
    let mut result = FxHashMap::default();
    for (minified, originals) in var_map {
        let unique: Vec<_> = originals
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        if unique.len() == 1 {
            result.insert(minified, unique[0].clone());
        }
    }

    result
}

/// Finds the lazy module wrappers in `code`.
///
/// A wrapper is a variable declarator `X = helper(…)` (in any
/// `var`/`let`/`const`, also later in a declarator chain) or an assignment
/// `X = helper(…)` whose callee resolves to a `__commonJS` or `__esm` helper
/// that `detect_bundle_runtime` identifies.
///
/// Callees are matched by resolved symbol, so a local that shadows a helper's
/// name is not a module. A wrapper nested inside another is part of the outer
/// module and is not reported separately.
#[must_use]
pub fn find_module_wrappers(code: &str) -> Vec<ModuleInfo> {
    lazy_modules(code).1
}

fn lazy_modules(code: &str) -> (Option<Bundler>, Vec<ModuleInfo>) {
    with_bundle_runtime(code, |program, scoping, collector| {
        let helpers = collector
            .helper_symbols()
            .iter()
            .filter_map(|(&symbol, helper)| match helper {
                BunHelper::CommonJs => Some((symbol, ModuleType::CommonJs)),
                BunHelper::Esm => Some((symbol, ModuleType::Esm)),
                _ => None,
            })
            .collect();
        let mut finder = WrapperFinder {
            scoping,
            helpers,
            source: code,
            modules: Vec::new(),
        };
        if !finder.helpers.is_empty() {
            finder.visit_program(program);
        }
        let mut modules = finder.modules;
        modules.sort_by_key(|m| (m.start, std::cmp::Reverse(m.end)));
        let mut last_end = 0;
        modules.retain(|m| {
            let outermost = m.start >= last_end;
            if outermost {
                last_end = m.end;
            }
            outermost
        });
        (collector.bundler(), modules)
    })
}

struct WrapperFinder<'s> {
    scoping: &'s Scoping,
    helpers: FxHashMap<SymbolId, ModuleType>,
    source: &'s str,
    modules: Vec<ModuleInfo>,
}

impl WrapperFinder<'_> {
    /// The module type when `expr` calls a lazy-module helper.
    fn wrapper_call<'e, 'a>(&self, expr: &'e Expression<'a>) -> Option<(ModuleType, &'e CallExpression<'a>)> {
        let Expression::CallExpression(call) = expr.without_parentheses() else {
            return None;
        };
        let Expression::Identifier(callee) = &call.callee else {
            return None;
        };
        let symbol = self.scoping.get_reference(callee.reference_id()).symbol_id()?;
        Some((*self.helpers.get(&symbol)?, call))
    }

    fn push(&mut self, name: String, module_type: ModuleType, start: u32, call: &CallExpression<'_>) {
        let body_start = call.arguments.first().map_or(call.span.end - 1, |arg| arg.span().start);
        self.modules.push(ModuleInfo {
            name,
            module_type,
            start: start as usize,
            end: call.span.end as usize,
            body_start: body_start as usize,
        });
    }
}

impl<'a> Visit<'a> for WrapperFinder<'_> {
    fn visit_variable_declaration(&mut self, it: &VariableDeclaration<'a>) {
        for (index, declarator) in it.declarations.iter().enumerate() {
            if let BindingPattern::BindingIdentifier(id) = &declarator.id
                && let Some(init) = &declarator.init
                && let Some((module_type, call)) = self.wrapper_call(init)
            {
                // The first declarator's module starts at the keyword.
                let start = if index == 0 {
                    it.span.start
                } else {
                    declarator.span.start
                };
                self.push(id.name.to_string(), module_type, start, call);
            }
        }
        walk::walk_variable_declaration(self, it);
    }

    fn visit_assignment_expression(&mut self, it: &AssignmentExpression<'a>) {
        if it.operator == AssignmentOperator::Assign
            && let Some((module_type, call)) = self.wrapper_call(&it.right)
        {
            let name = match &it.left {
                AssignmentTarget::AssignmentTargetIdentifier(id) => id.name.to_string(),
                target => target.span().source_text(self.source).to_string(),
            };
            self.push(name, module_type, it.span.start, call);
        }
        walk::walk_assignment_expression(self, it);
    }
}

/// Split a Bun bundle into individual modules.
/// Returns the runtime code, individual modules, and main/entry code.
pub struct BundleSplit {
    /// The bundler the runtime helpers identify.
    pub bundler: Option<Bundler>,
    pub runtime: String,
    pub modules: Vec<(ModuleInfo, String)>,
    pub main: String,
}

#[must_use]
pub fn split_bundle(code: &str) -> BundleSplit {
    let (bundler, modules) = lazy_modules(code);

    if modules.is_empty() {
        return BundleSplit {
            bundler,
            runtime: String::new(),
            modules: Vec::new(),
            main: code.to_string(),
        };
    }

    // Runtime is everything before first module
    let runtime = code[..modules[0].start].to_string();

    // Extract each module's code
    let mut module_codes = Vec::new();
    for module in &modules {
        let module_code = code[module.start..module.end].to_string();
        module_codes.push((module.clone(), module_code));
    }

    // Main is everything after last module
    let last_end = modules.last().map_or(0, |m| m.end);
    let main = code[last_end..].to_string();

    BundleSplit {
        bundler,
        runtime,
        modules: module_codes,
        main,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_mr_exports() {
        let code = r"MR(target, { createClient: () => abc, fetchData: () => xyz });";
        let exports = extract_mr_exports(code);
        assert_eq!(exports.len(), 2);
        assert!(
            exports
                .iter()
                .any(|e| e.minified_name == "abc" && e.original_name == "createClient")
        );
        assert!(
            exports
                .iter()
                .any(|e| e.minified_name == "xyz" && e.original_name == "fetchData")
        );
    }

    #[test]
    fn test_extract_this_name() {
        let code = r#"class Abc extends Error { constructor() { super(); this.name = "ValidationError"; } }"#;
        let names = extract_this_name_patterns(code);
        assert_eq!(names.len(), 1);
        assert_eq!(names[0].minified_name, "Abc");
        assert_eq!(names[0].original_name, "ValidationError");
    }

    #[test]
    fn test_extract_display_name() {
        let code = r#"xyz.displayName = "MyComponent";"#;
        let names = extract_display_name_patterns(code);
        assert_eq!(names.len(), 1);
        assert_eq!(names[0].minified_name, "xyz");
        assert_eq!(names[0].original_name, "MyComponent");
    }

    const RUNTIME: &str = "var y = (a, b) => () => (b || a((b = {exports: {}}).exports, b), b.exports);
var h = (a, b) => () => (a && (b = a(a = 0)), b);
";

    fn wrappers(tail: &str) -> Vec<(String, ModuleType)> {
        find_module_wrappers(&format!("{RUNTIME}{tail}"))
            .into_iter()
            .map(|m| (m.name, m.module_type))
            .collect()
    }

    fn named(names: &[(&str, ModuleType)]) -> Vec<(String, ModuleType)> {
        names.iter().map(|(n, t)| ((*n).to_string(), *t)).collect()
    }

    #[test]
    fn test_find_module_wrappers() {
        let code = format!(
            "{RUNTIME}var abc = y((exports) => {{ exports.foo = 1; }});\nvar xyz = h(() => {{ return 42; }});\n"
        );
        let modules = find_module_wrappers(&code);
        assert_eq!(modules.len(), 2);
        assert_eq!(modules[0].name, "abc");
        assert_eq!(modules[0].module_type, ModuleType::CommonJs);
        assert_eq!(
            &code[modules[0].start..modules[0].end],
            "var abc = y((exports) => { exports.foo = 1; })"
        );
        assert_eq!(modules[1].name, "xyz");
        assert_eq!(modules[1].module_type, ModuleType::Esm);
    }

    #[test]
    fn finds_every_wrapper_in_a_declarator_chain() {
        assert_eq!(
            wrappers("var a = y((e, m) => { m.exports = 1; }), b = h(() => {}), c = 1, d = y((e) => {});"),
            named(&[
                ("a", ModuleType::CommonJs),
                ("b", ModuleType::Esm),
                ("d", ModuleType::CommonJs)
            ])
        );
    }

    #[test]
    fn finds_wrappers_declared_with_let_and_const() {
        assert_eq!(
            wrappers("let a = y((e) => {}); const b = h(() => {}), c = y((e) => {});"),
            named(&[
                ("a", ModuleType::CommonJs),
                ("b", ModuleType::Esm),
                ("c", ModuleType::CommonJs)
            ])
        );
    }

    #[test]
    fn finds_wrappers_assigned_to_existing_bindings() {
        let code = format!("{RUNTIME}var a, b; a = y((e) => {{}}); b = h(() => {{}}), o.c = h(() => {{}});");
        let modules = find_module_wrappers(&code);
        let names: Vec<_> = modules.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["a", "b", "o.c"]);
        assert_eq!(&code[modules[0].start..modules[0].end], "a = y((e) => {})");
    }

    #[test]
    fn locals_that_shadow_a_helper_name_are_not_modules() {
        assert_eq!(
            wrappers(
                "function f(y) { var a = y(() => {}); return a; }
                 { let h = (x) => x; var b = h(() => {}); }
                 var c = y((e) => {});"
            ),
            named(&[("c", ModuleType::CommonJs)])
        );
    }

    #[test]
    fn a_wrapper_nested_in_another_belongs_to_the_outer_module() {
        assert_eq!(
            wrappers("var outer = y((e, m) => { var inner = h(() => {}); m.exports = inner; });"),
            named(&[("outer", ModuleType::CommonJs)])
        );
    }

    #[test]
    fn module_spans_end_at_the_call_regardless_of_parens_in_literals() {
        let code = format!("{RUNTIME}var a = y((e) => {{ e.re = /\\)/; e.s = `)${{'('}}`; }}); var b = h(() => {{}});");
        let modules = find_module_wrappers(&code);
        assert_eq!(modules.len(), 2);
        assert!(
            code[modules[0].start..modules[0].end].ends_with("`; })"),
            "{:?}",
            &code[modules[0].start..modules[0].end]
        );
    }

    #[test]
    fn calls_to_functions_that_are_not_wrapper_helpers_are_not_modules() {
        // `y` is an ordinary function, not a `__commonJS` helper.
        let code = "function y(e){let n=new Set(e);return t=>n.has(t)}var Ee=y(Object.keys(ge));var h=y([1]);";
        assert!(find_module_wrappers(code).is_empty());
    }
}
