//! Post-codegen annotation of the esbuild / Bun runtime: a header naming the
//! bundler and the detected runtime helpers, and a comment above each lazy
//! `__esm` / `__commonJS` module wrapper.

use std::fmt::Write as _;

use regex::Regex;

use super::runtime::{BunHelper, BunHelperMap, Bundler};

/// Annotates `code` with the runtime `helpers` (current name → helper, as
/// detected by `BunRuntimeCollector`) and the `bundler` whose runtime variant
/// they are. A bundle built by another tool can embed that runtime through a
/// prebuilt dependency, so the header names the runtime, not the build tool.
#[must_use]
pub fn annotate_runtime(code: &str, helpers: &BunHelperMap, bundler: Option<Bundler>) -> String {
    if helpers.is_empty() {
        return code.to_string();
    }

    let (hashbang, body) = match code.strip_prefix("#!") {
        Some(_) => code.split_at(code.find('\n').map_or(code.len(), |newline| newline + 1)),
        None => ("", code),
    };

    let mut result = String::with_capacity(code.len() + 4096);
    result.push_str(hashbang);
    result.push_str(&runtime_header(helpers, bundler));

    let wrappers: Vec<String> = helpers
        .iter()
        .filter(|(_, helper)| matches!(helper, BunHelper::Esm | BunHelper::CommonJs))
        .map(|(name, _)| regex::escape(name))
        .collect();
    let re = (!wrappers.is_empty())
        .then(|| {
            Regex::new(&format!(
                r"^var\s+([A-Za-z_$][\w$]*)\s*=\s*({})\s*\(",
                wrappers.join("|")
            ))
        })
        .and_then(Result::ok);
    let label = bundler.map_or(String::new(), |bundler| format!("{} ", bundler.name()));

    let mut count = 0u32;
    for line in body.lines() {
        if let Some(caps) = re.as_ref().and_then(|re| re.captures(line.trim_start())) {
            let kind = match helpers.get(&caps[2]) {
                Some(BunHelper::Esm) => "ESM",
                _ => "CommonJS",
            };
            result.push_str("// ═══════════════════════════════════════\n");
            let _ = writeln!(result, "// {label}{kind} module: {}", &caps[1]);
            result.push_str("// ═══════════════════════════════════════\n");
            count += 1;
        }
        result.push_str(line);
        result.push('\n');
    }

    if count > 0 {
        eprintln!("[DEOBFUSCATE] Annotated {count} lazy module wrappers");
    }
    result
}

fn runtime_header(helpers: &BunHelperMap, bundler: Option<Bundler>) -> String {
    let mut entries: Vec<(&str, &str)> = helpers
        .iter()
        .map(|(name, helper)| (helper.canonical(), name.as_str()))
        .collect();
    entries.sort_unstable();

    let mut header = String::from("// ═══ Bundler runtime ═══\n");
    if let Some(bundler) = bundler {
        let _ = writeln!(header, "// Runtime: {}", bundler.name());
    }
    header.push_str("// Helpers:\n");
    for (canonical, name) in entries {
        let _ = writeln!(header, "//   {name} → {canonical}");
    }
    header.push('\n');
    header
}

#[cfg(test)]
mod tests {
    use super::*;

    fn helpers(entries: &[(&str, BunHelper)]) -> BunHelperMap {
        entries
            .iter()
            .map(|(name, helper)| ((*name).to_string(), *helper))
            .collect()
    }

    #[test]
    fn labels_modules_with_the_detected_bundler() {
        let map = helpers(&[("E", BunHelper::Esm), ("C", BunHelper::CommonJs)]);
        let code = "var a = E(() => {});\nvar b = C((exports, module) => {});\n";
        let out = annotate_runtime(code, &map, Some(Bundler::Esbuild));
        assert!(out.contains("// Runtime: esbuild\n"), "{out}");
        assert!(
            out.contains("//   E → __esm\n") && out.contains("//   C → __commonJS\n"),
            "{out}"
        );
        assert!(out.contains("// esbuild ESM module: a\n"), "{out}");
        assert!(out.contains("// esbuild CommonJS module: b\n"), "{out}");
    }

    #[test]
    fn omits_the_bundler_when_unknown() {
        let map = helpers(&[("E", BunHelper::Esm)]);
        let out = annotate_runtime("var a = E(() => {});\n", &map, None);
        assert!(!out.contains("Runtime:"), "{out}");
        assert!(out.contains("// ESM module: a\n"), "{out}");
    }

    #[test]
    fn annotates_only_calls_to_detected_wrappers() {
        let map = helpers(&[("E", BunHelper::Esm)]);
        let out = annotate_runtime("var a = B(() => {});\nvar b = Ex(() => {});\n", &map, None);
        assert!(!out.contains("module:"), "{out}");
    }

    #[test]
    fn keeps_the_hashbang_first() {
        let map = helpers(&[("E", BunHelper::Esm)]);
        let out = annotate_runtime("#!/usr/bin/env node\nvar a = E(() => {});\n", &map, Some(Bundler::Bun));
        assert!(
            out.starts_with("#!/usr/bin/env node\n// ═══ Bundler runtime ═══\n"),
            "{out}"
        );
    }

    #[test]
    fn leaves_code_without_helpers_untouched() {
        let code = "var a = E(() => {});";
        assert_eq!(annotate_runtime(code, &BunHelperMap::default(), None), code);
    }
}
