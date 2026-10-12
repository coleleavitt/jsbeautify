use regex::Regex;
use std::fmt::Write as _;
use std::sync::LazyLock;

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
    if !code.contains("__export(") {
        return (code.to_string(), 0);
    }
    let mut result = String::with_capacity(code.len() + 4096);
    let mut count = 0usize;
    let mut last_end = 0;

    for m in EXPORT_CALL_RE.find_iter(code) {
        let Some(caps) = EXPORT_CALL_RE.captures(m.as_str()) else {
            continue;
        };
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
        let _ = write!(result, "/* module {target}: {{ {desc} }} */ ");
        result.push_str(m.as_str());
        last_end = m.end();
        count += 1;
    }

    result.push_str(&code[last_end..]);
    (result, count)
}
