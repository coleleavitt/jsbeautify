//! Bun-specific name extraction and module splitting.
//!
//! Extracts original names from Bun bundle patterns:
//! - MR() export mappings: `MR(target, { exportName: () => minifiedVar })`
//! - this.name patterns: `this.name = "ClassName"` in class constructors
//! - displayName patterns: `X.displayName = "ComponentName"`
//! - Module wrappers: `y((exports, module) => { ... })` and `h(() => { ... })`

use regex::Regex;
use rustc_hash::FxHashMap;
use std::sync::LazyLock;

/// Extracted name mapping from MR() export calls
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

/// Extract all MR() export mappings from code.
/// Returns mappings from minified variable name to original export name.
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

#[must_use]
pub fn detect_wrapper_functions(code: &str) -> (String, String) {
    let preamble = &code[..5000.min(code.len())];

    // __commonJS: (a,b)=>()=>(b||a((b={exports:{}}).exports,b),b.exports)
    // Match structure without backreferences - look for the {exports:{}} pattern
    let factory_re = Regex::new(
        r"(?:,|var\s+)([A-Za-z_$][A-Za-z0-9_$]*)\s*=\s*\([A-Za-z_$][A-Za-z0-9_$]*,\s*[A-Za-z_$][A-Za-z0-9_$]*\)\s*=>\s*\(\)\s*=>\s*\([^)]*\{\s*exports\s*:\s*\{\s*\}\s*\}"
    ).expect("valid regex pattern");

    let factory_fn = factory_re
        .captures(preamble)
        .map(|c| {
            c.get(1)
                .expect("capture group 1 guaranteed by regex")
                .as_str()
                .to_string()
        })
        .unwrap_or_else(|| "y".to_string());

    // __esm: (a,b)=>()=>(a&&(b=a(a=0)),b)
    // Match structure - look for the (a=0) pattern
    let lazy_re = Regex::new(
        r"(?:,|var\s+)([A-Za-z_$][A-Za-z0-9_$]*)\s*=\s*\([A-Za-z_$][A-Za-z0-9_$]*,\s*[A-Za-z_$][A-Za-z0-9_$]*\)\s*=>\s*\(\)\s*=>\s*\([^)]*=\s*0\s*\)"
    ).expect("valid regex pattern");

    let lazy_fn = lazy_re
        .captures(preamble)
        .map(|c| {
            c.get(1)
                .expect("capture group 1 guaranteed by regex")
                .as_str()
                .to_string()
        })
        .unwrap_or_else(|| "h".to_string());

    (factory_fn, lazy_fn)
}

/// Find all module wrappers in code.
/// Returns list of modules with their positions.
#[must_use]
pub fn find_module_wrappers(code: &str) -> Vec<ModuleInfo> {
    let (mod_fn, laz_fn) = detect_wrapper_functions(code);
    let mut modules = Vec::new();

    // CJS modules: var X = y(...)
    let mod_re = Regex::new(&format!(
        r"var\s+([A-Za-z0-9_$]+)\s*=\s*{}\s*\(",
        regex::escape(&mod_fn)
    ))
    .expect("valid regex pattern");

    for cap in mod_re.captures_iter(code) {
        let name = cap
            .get(1)
            .expect("capture group 1 guaranteed by regex")
            .as_str()
            .to_string();
        let decl_start = cap.get(0).expect("capture group 0 guaranteed by regex").start();
        let factory_start = cap.get(0).expect("capture group 0 guaranteed by regex").end();

        // Find matching closing paren
        if let Some(end) = find_matching_paren(code, factory_start - 1) {
            modules.push(ModuleInfo {
                name,
                module_type: ModuleType::CommonJs,
                start: decl_start,
                end,
                body_start: factory_start,
            });
        }
    }

    // ESM modules: var X = h(...)
    let laz_re = Regex::new(&format!(
        r"var\s+([A-Za-z0-9_$]+)\s*=\s*{}\s*\(",
        regex::escape(&laz_fn)
    ))
    .expect("valid regex pattern");

    for cap in laz_re.captures_iter(code) {
        let name = cap
            .get(1)
            .expect("capture group 1 guaranteed by regex")
            .as_str()
            .to_string();
        let decl_start = cap.get(0).expect("capture group 0 guaranteed by regex").start();
        let factory_start = cap.get(0).expect("capture group 0 guaranteed by regex").end();

        if let Some(end) = find_matching_paren(code, factory_start - 1) {
            modules.push(ModuleInfo {
                name,
                module_type: ModuleType::Esm,
                start: decl_start,
                end,
                body_start: factory_start,
            });
        }
    }

    // Sort by position
    modules.sort_by_key(|m| m.start);
    modules
}

fn find_matching_paren(code: &str, start: usize) -> Option<usize> {
    let bytes = code.as_bytes();
    if start >= bytes.len() || bytes[start] != b'(' {
        return None;
    }

    let mut depth = 1;
    let mut i = start + 1;
    let mut in_string = false;
    let mut string_char = 0u8;

    while i < bytes.len() && depth > 0 {
        let ch = bytes[i];

        if in_string {
            if ch == string_char && (i == 0 || bytes[i - 1] != b'\\') {
                in_string = false;
            }
        } else {
            match ch {
                b'"' | b'\'' | b'`' => {
                    in_string = true;
                    string_char = ch;
                }
                b'(' => depth += 1,
                b')' => depth -= 1,
                _ => {}
            }
        }
        i += 1;
    }

    if depth == 0 { Some(i) } else { None }
}

/// Split a Bun bundle into individual modules.
/// Returns the runtime code, individual modules, and main/entry code.
pub struct BundleSplit {
    pub runtime: String,
    pub modules: Vec<(ModuleInfo, String)>,
    pub main: String,
}

#[must_use]
pub fn split_bundle(code: &str) -> BundleSplit {
    let modules = find_module_wrappers(code);

    if modules.is_empty() {
        return BundleSplit {
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
    let last_end = modules.last().map(|m| m.end).unwrap_or(0);
    let main = code[last_end..].to_string();

    BundleSplit {
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
        let code = r#"MR(target, { createClient: () => abc, fetchData: () => xyz });"#;
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

    #[test]
    fn test_find_module_wrappers() {
        let code = r#"
var y = (a, b) => () => (b || a((b = {exports: {}}).exports, b), b.exports);
var h = (a, b) => () => (a && (b = a(a = 0)), b);
var abc = y((exports) => { exports.foo = 1; });
var xyz = h(() => { return 42; });
"#;
        let modules = find_module_wrappers(code);
        assert_eq!(modules.len(), 2);
        assert_eq!(modules[0].name, "abc");
        assert_eq!(modules[0].module_type, ModuleType::CommonJs);
        assert_eq!(modules[1].name, "xyz");
        assert_eq!(modules[1].module_type, ModuleType::Esm);
    }
}
