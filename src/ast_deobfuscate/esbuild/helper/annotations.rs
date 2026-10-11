use super::{BundleType, EsbuildHelperCollector, EsbuildHelperKind, SupportHelperKind};
use std::fmt::Write as _;

use super::annotation_names::helper_display_name;

#[must_use]
pub fn annotate_esbuild_modules(code: &str, collector: &EsbuildHelperCollector) -> String {
    let helpers = collector.get_helpers();
    let module_wrappers = collector.get_module_wrappers();

    if helpers.is_empty() && module_wrappers.is_empty() {
        return code.to_string();
    }

    let commonjs_helpers: Vec<&str> = helpers
        .iter()
        .filter(|(_, info)| matches!(info.kind, EsbuildHelperKind::CommonJs))
        .map(|(name, _)| name.as_str())
        .collect();

    let esm_helpers: Vec<&str> = helpers
        .iter()
        .filter(|(_, info)| matches!(info.kind, EsbuildHelperKind::Esm))
        .map(|(name, _)| name.as_str())
        .collect();

    let mut result = String::with_capacity(code.len() + 8192);

    // Extract hashbang if present at start — must stay first for valid JS
    let (hashbang, code_after_hashbang) = code.starts_with("#!").then_some(()).map_or((None, code), |()| {
        code.find('\n').map_or((Some(code), ""), |newline_pos| {
            let hashbang_line = &code[..=newline_pos];
            let rest = &code[newline_pos + 1..];
            (Some(hashbang_line), rest)
        })
    });

    // Hashbang MUST come first (ECMAScript spec requirement)
    if let Some(hashbang_line) = hashbang {
        result.push_str(hashbang_line);
    }

    // Header goes after hashbang
    result.push_str(&generate_helper_header(collector));

    let mut count = 0u32;

    for line in code_after_hashbang.lines() {
        let trimmed = line.trim_start();

        for helper in &commonjs_helpers {
            let pattern = format!(" = {helper}((");
            if let Some(rest) = trimmed.strip_prefix("var ")
                && let Some(name_end) = rest.find(&pattern)
            {
                let module_name = &rest[..name_end];
                if !module_name.is_empty() && module_name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$') {
                    result.push_str("// ═══════════════════════════════════════\n");
                    result.push_str("// esbuild CommonJS Module: ");
                    result.push_str(module_name);
                    result.push('\n');
                    result.push_str("// ═══════════════════════════════════════\n");
                    count = count.wrapping_add(1);
                }
            }
        }

        for helper in &esm_helpers {
            let pattern = format!(" = {helper}({{");
            let pattern_alt = format!(" = {helper}((");
            if let Some(rest) = trimmed.strip_prefix("var ") {
                let found = rest.find(&pattern).or_else(|| rest.find(&pattern_alt));
                if let Some(name_end) = found {
                    let module_name = &rest[..name_end];
                    if !module_name.is_empty()
                        && module_name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$')
                    {
                        result.push_str("// ═══════════════════════════════════════\n");
                        result.push_str("// esbuild ESM Module: ");
                        result.push_str(module_name);
                        result.push('\n');
                        result.push_str("// ═══════════════════════════════════════\n");
                        count = count.wrapping_add(1);
                    }
                }
            }
        }

        result.push_str(line);
        result.push('\n');
    }

    if count > 0 {
        eprintln!("[ESBUILD] Annotated {count} esbuild modules");
    }

    result
}

fn generate_helper_header(collector: &EsbuildHelperCollector) -> String {
    let helpers = collector.get_helpers();
    let bundle_type = collector.get_bundle_type();
    let support_helpers = collector.get_support_helpers();

    if helpers.is_empty() {
        return String::new();
    }

    let mut header = String::with_capacity(2048);

    header.push_str("// ╔═══════════════════════════════════════════════════════════════════╗\n");
    header.push_str("// ║                    ESBUILD BUNDLE ANALYSIS                        ║\n");
    header.push_str("// ╠═══════════════════════════════════════════════════════════════════╣\n");

    match bundle_type {
        BundleType::Anthropic => {
            header.push_str("// ║  Bundle Type: ANTHROPIC-MODIFIED ESBUILD                         ║\n");
            header.push_str("// ║  (Claude Code / Anthropic CLI build system)                     ║\n");
        }
        BundleType::Standard => {
            header.push_str("// ║  Bundle Type: STANDARD ESBUILD                                   ║\n");
        }
        BundleType::Unknown => {
            header.push_str("// ║  Bundle Type: UNKNOWN                                            ║\n");
        }
    }

    header.push_str("// ╠═══════════════════════════════════════════════════════════════════╣\n");
    header.push_str("// ║  Helper Mapping:                                                  ║\n");

    for (name, info) in helpers {
        let semantic_name = helper_display_name(info.kind);
        let modified = if info.is_anthropic_modified() {
            " [MODIFIED]"
        } else {
            ""
        };

        let _ = writeln!(header, "// ║    {name} → {semantic_name}{modified}");
    }

    if !support_helpers.is_empty() {
        header.push_str("// ╠═══════════════════════════════════════════════════════════════════╣\n");
        header.push_str("// ║  Anthropic Support Helpers:                                       ║\n");
        for (name, kind) in support_helpers {
            let desc = match kind {
                SupportHelperKind::GetterBind => "function(q) { return this[q] }",
                SupportHelperKind::Identity => "(q) => q",
                SupportHelperKind::SetterBind => "function(q, K) { this[q] = ... }",
            };
            let _ = writeln!(header, "// ║    {name}: {desc}");
        }
    }

    header.push_str("// ╚═══════════════════════════════════════════════════════════════════╝\n");
    header.push('\n');

    header
}
