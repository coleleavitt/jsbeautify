//! Bun module boundary detection and annotation
//!
//! Detects Bun's lazy module wrapper patterns and annotates boundaries with comments.
//! This is a post-codegen string pass similar to the webpack module annotator.
//!
//! Bun patterns detected:
//! - `var MODULE_NAME = E(() => { ... })` where E is `__esm` helper
//! - `var MODULE_NAME = V(() => { ... })` (minified form)
//! - `var MODULE_NAME = B(() => { ... })` where B is `__commonJS` helper

use regex::Regex;

/// Annotates Bun module boundaries in the generated code.
///
/// This function scans the output code for Bun's lazy module wrapper patterns
/// and inserts comment annotations to mark module boundaries.
///
/// # Arguments
/// * `code` - The generated JavaScript code
/// * `esm_helpers` - Names of detected `__esm` helper functions
/// * `cjs_helpers` - Names of detected `__commonJS` helper functions
///
/// # Returns
/// The annotated code with module boundary comments
#[must_use]
pub fn annotate_bun_modules(code: &str, esm_helpers: &[String], cjs_helpers: &[String]) -> String {
    let mut result = String::with_capacity(code.len() + 4096);
    let mut count = 0u32;

    let helper_pattern = build_helper_pattern(esm_helpers, cjs_helpers);
    let Ok(re) = Regex::new(&helper_pattern) else {
        return code.to_string();
    };

    for line in code.lines() {
        let trimmed = line.trim_start();

        if let Some(caps) = re.captures(trimmed)
            && let Some(module_name) = caps.get(1)
        {
            let name = module_name.as_str();
            if is_valid_module_name(name) {
                result.push_str("// ═══════════════════════════════════════\n");
                result.push_str("// Bun Module: ");
                result.push_str(name);
                result.push('\n');
                result.push_str("// ═══════════════════════════════════════\n");
                count = count.wrapping_add(1);
            }
        }

        result.push_str(line);
        result.push('\n');
    }

    if count > 0 {
        eprintln!("[DEOBFUSCATE] Annotated {count} Bun modules");
    }

    result
}

fn build_helper_pattern(esm_helpers: &[String], cjs_helpers: &[String]) -> String {
    let mut helpers: Vec<&str> = Vec::new();

    for h in esm_helpers {
        helpers.push(h.as_str());
    }
    for h in cjs_helpers {
        helpers.push(h.as_str());
    }

    if helpers.is_empty() {
        helpers.push("__esm");
        helpers.push("__commonJS");
        helpers.push("E");
        helpers.push("V");
        helpers.push("B");
        helpers.push("L");
    }

    let escaped: Vec<String> = helpers.iter().map(|h| regex::escape(h)).collect();
    let helper_group = escaped.join("|");

    format!(r"^var\s+([a-zA-Z_$][a-zA-Z0-9_$]*)\s*=\s*(?:{helper_group})\s*\(\s*\(\s*\)\s*=>")
}

fn is_valid_module_name(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }

    let first = name.chars().next().expect("non-empty module name");
    if !first.is_ascii_alphabetic() && first != '_' && first != '$' {
        return false;
    }

    name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_annotate_esm_module() {
        let code = "var myModule = E(() => {\n  console.log('hello');\n});";
        let result = annotate_bun_modules(code, &[], &[]);

        assert!(result.contains("Bun Module: myModule"), "Should annotate module");
        assert!(result.contains("═══"), "Should have separator");
    }

    #[test]
    fn test_annotate_commonjs_module() {
        let code = "var myModule = B(() => {\n  module.exports = {};\n});";
        let result = annotate_bun_modules(code, &[], &[]);

        assert!(result.contains("Bun Module: myModule"), "Should annotate module");
    }

    #[test]
    fn test_annotate_with_custom_helpers() {
        let code = "var myModule = customEsm(() => {\n  console.log('hello');\n});";
        let result = annotate_bun_modules(code, &["customEsm".to_string()], &[]);

        assert!(
            result.contains("Bun Module: myModule"),
            "Should annotate with custom helper"
        );
    }

    #[test]
    fn test_no_annotation_for_regular_code() {
        let code = "var x = 42;\nvar y = function() { return x; };";
        let result = annotate_bun_modules(code, &[], &[]);

        assert!(!result.contains("Bun Module"), "Should not annotate regular code");
    }

    #[test]
    fn test_multiple_modules() {
        let code = "var mod1 = E(() => {});\nvar mod2 = E(() => {});\nvar mod3 = B(() => {});";
        let result = annotate_bun_modules(code, &[], &[]);

        assert!(result.contains("Bun Module: mod1"), "Should annotate mod1");
        assert!(result.contains("Bun Module: mod2"), "Should annotate mod2");
        assert!(result.contains("Bun Module: mod3"), "Should annotate mod3");
    }

    #[test]
    fn test_valid_module_names() {
        assert!(is_valid_module_name("myModule"));
        assert!(is_valid_module_name("_private"));
        assert!(is_valid_module_name("$jquery"));
        assert!(is_valid_module_name("mod123"));
        assert!(!is_valid_module_name(""));
        assert!(!is_valid_module_name("123abc"));
    }
}
