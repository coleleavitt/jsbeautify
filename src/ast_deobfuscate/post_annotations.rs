//! TypeScript enum IIFE annotation + namespace module naming + dynamic import annotation
//!
//! Three post-codegen string-level annotation passes:
//!
//! 1. **Enum annotation**: Detects TypeScript-compiled enum IIFEs:
//!    `(function(H) { H.Foo = "bar"; ... })(X || (X = {}));`
//!    and adds `/* enum EnumName { Foo="bar", ... } */` above them.
//!
//! 2. **Namespace module naming**: For `__export(CjH, { name1: ..., name2: ... })`,
//!    derives a module name from the first export and annotates with a comment.
//!
//! 3. **Arrow sequence expansion**: Converts expression-body arrows containing
//!    sequence expressions into block bodies for readability.

use regex::Regex;
use std::sync::LazyLock;

// ─────────────────────────────────────────────────────────────────────────────
// 1. TypeScript Enum IIFE Annotation
// ─────────────────────────────────────────────────────────────────────────────

static ENUM_IIFE_RE: LazyLock<Regex> = LazyLock::new(|| {
    // Matches: (function(H) { ... })(X.Name || (X.Name = {}));
    // or: (function(H) { ... })(X || (X = {}));
    Regex::new(
        r"(?m)^\s*\(function\([A-Za-z$_]+\) \{([^}]*(?:\{[^}]*\}[^}]*)*)\}\)\(([^)]+) \|\| \(([^)]+) = \{\}\)\);?",
    )
    .expect("valid enum IIFE regex")
});

static ENUM_ASSIGN_RE: LazyLock<Regex> = LazyLock::new(|| {
    // Matches: H.Name = "value" or H.Name = 0 or H[H.Name = 0] = "Name"
    Regex::new(r#"[A-Za-z$_]+\.([A-Za-z_$][A-Za-z0-9_$]*)\s*=\s*("[^"]*"|[0-9]+)"#).expect("valid enum assign regex")
});

/// Annotates TypeScript enum IIFEs with a summary comment.
#[must_use]
pub fn annotate_ts_enums(code: &str) -> (String, usize) {
    let mut result = String::with_capacity(code.len() + 4096);
    let mut count = 0usize;
    let mut last_end = 0;

    for m in ENUM_IIFE_RE.find_iter(code) {
        let full = m.as_str();
        let caps = ENUM_IIFE_RE.captures(full).unwrap();
        let body = caps.get(1).map_or("", |m| m.as_str());
        let name_expr = caps.get(2).map_or("", |m| m.as_str());

        // Extract enum name from the expression (e.g. "lM.HttpAuthLocation" → "HttpAuthLocation")
        let enum_name = name_expr.rsplit('.').next().unwrap_or(name_expr).trim();

        // Extract members from the body
        let members: Vec<String> = ENUM_ASSIGN_RE
            .captures_iter(body)
            .map(|c| {
                let name = c.get(1).map_or("", |m| m.as_str());
                let value = c.get(2).map_or("", |m| m.as_str());
                format!("{name}={value}")
            })
            .collect();

        if !members.is_empty() && !enum_name.is_empty() && enum_name.len() > 1 {
            result.push_str(&code[last_end..m.start()]);
            let summary = if members.len() <= 6 {
                members.join(", ")
            } else {
                let first_five = members[..5].join(", ");
                format!("{first_five}, ... ({} more)", members.len() - 5)
            };
            result.push_str(&format!("/* enum {enum_name} {{ {summary} }} */\n"));
            result.push_str(full);
            last_end = m.end();
            count += 1;
        }
    }

    result.push_str(&code[last_end..]);
    (result, count)
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. Namespace Module Naming (post-export annotation)
// ─────────────────────────────────────────────────────────────────────────────

static EXPORT_CALL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"__export\(([A-Za-z$_][A-Za-z0-9$_]*),\s*\{\s*([^}]+)\}\)").expect("valid export regex")
});

static FIRST_EXPORT_NAME_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([a-zA-Z_$][a-zA-Z0-9_$]*):\s*\(\)").expect("valid first export name regex"));

/// Annotates `__export(target, { ... })` calls with a comment showing the module's purpose.
/// Derives a module name from the first non-generic export.
#[must_use]
pub fn annotate_namespace_modules(code: &str) -> (String, usize) {
    let mut result = String::with_capacity(code.len() + 4096);
    let mut count = 0usize;
    let mut last_end = 0;

    for m in EXPORT_CALL_RE.find_iter(code) {
        let caps = EXPORT_CALL_RE.captures(m.as_str()).unwrap();
        let target = caps.get(1).map_or("", |m| m.as_str());
        let exports_body = caps.get(2).map_or("", |m| m.as_str());

        // Get all export names
        let names: Vec<&str> = FIRST_EXPORT_NAME_RE
            .captures_iter(exports_body)
            .filter_map(|c| c.get(1).map(|m| m.as_str()))
            .filter(|n| !matches!(*n, "default" | "init" | "get" | "set"))
            .collect();

        if names.is_empty() || target.len() > 20 {
            continue;
        }

        // Derive module description from exports
        let desc = if names.len() == 1 {
            names[0].to_string()
        } else if names.len() <= 4 {
            names.join(", ")
        } else {
            format!("{}, {} + {} more", names[0], names[1], names.len() - 2)
        };

        result.push_str(&code[last_end..m.start()]);
        result.push_str(&format!("/* module {target}: {{ {desc} }} */ "));
        result.push_str(m.as_str());
        last_end = m.end();
        count += 1;
    }

    result.push_str(&code[last_end..]);
    (result, count)
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. Arrow Sequence Expansion (post-codegen string replacement)
// ─────────────────────────────────────────────────────────────────────────────

static ARROW_SEQ_RE: LazyLock<Regex> = LazyLock::new(|| {
    // Matches: => (BODY) where BODY contains at least 2 top-level commas.
    // We capture everything inside the outermost parens.
    Regex::new(r"=>\s*\((.+)\)([;,]?)").expect("valid arrow seq regex")
});

/// Expands simple arrow sequences with 2+ top-level commas into block form.
/// `(H) => (a(), b(), c())` → `(H) => { a(); b(); return c(); }`
#[must_use]
pub fn expand_arrow_sequences(code: &str) -> (String, usize) {
    let mut count = 0usize;
    let result = ARROW_SEQ_RE.replace_all(code, |caps: &regex::Captures| {
        let body = caps.get(1).map_or("", |m| m.as_str());
        let trailing = caps.get(2).map_or("", |m| m.as_str());

        // Split on top-level commas (respecting paren nesting)
        let parts = split_top_level_commas(body);
        if parts.len() < 3 {
            return caps[0].to_string();
        }

        // Skip if any part looks complex (contains => or {})
        if parts.iter().any(|p| p.contains("=>") || p.contains('{')) {
            return caps[0].to_string();
        }

        count += 1;
        let mut out = String::from("=> {\n");
        for (i, part) in parts.iter().enumerate() {
            let trimmed = part.trim();
            if i == parts.len() - 1 {
                out.push_str(&format!("\t\treturn {trimmed};\n"));
            } else {
                out.push_str(&format!("\t\t{trimmed};\n"));
            }
        }
        out.push_str("\t}");
        out.push_str(trailing);
        out
    });

    (result.into_owned(), count)
}

/// Split a string on top-level commas (respecting parenthesis nesting).
fn split_top_level_commas(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0;
    let mut last = 0;

    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&s[last..i]);
                last = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&s[last..]);
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn annotates_enum_iife() {
        let code = r#"(function(H) { H.HEADER = "header"; H.QUERY = "query"; })(lM.HttpAuthLocation || (lM.HttpAuthLocation = {}));"#;
        let (out, count) = annotate_ts_enums(code);
        assert_eq!(count, 1);
        assert!(out.contains("/* enum HttpAuthLocation"), "Should annotate: {out}");
        assert!(out.contains("HEADER=\"header\""), "Should show members: {out}");
    }

    #[test]
    fn annotates_namespace_module() {
        let code = r#"__export(GZ8, { writeToStdout: () => UK, exitWithError: () => Ba4 })"#;
        let (out, count) = annotate_namespace_modules(code);
        assert_eq!(count, 1);
        assert!(out.contains("/* module GZ8:"), "Should annotate target: {out}");
        assert!(out.contains("writeToStdout"), "Should show export: {out}");
    }

    #[test]
    fn expands_arrow_sequence() {
        let code = "const f = (H) => (validate(H), transform(H), emit(H));";
        let (out, count) = expand_arrow_sequences(code);
        assert_eq!(count, 1);
        assert!(out.contains("validate(H);"), "Should split: {out}");
        assert!(out.contains("return emit(H);"), "Should have return: {out}");
    }
}
