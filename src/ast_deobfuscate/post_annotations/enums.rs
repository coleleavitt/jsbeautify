use regex::Regex;
use std::fmt::Write as _;
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
        let Some(caps) = ENUM_IIFE_RE.captures(full) else {
            continue;
        };
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
            let _ = writeln!(result, "/* enum {enum_name} {{ {summary} }} */");
            result.push_str(full);
            last_end = m.end();
            count += 1;
        }
    }

    result.push_str(&code[last_end..]);
    (result, count)
}
