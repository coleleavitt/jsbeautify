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

/// Expands simple arrow sequences with 2+ top-level commas into block form.
/// `(H) => (a(), b(), c())` → `(H) => { a(); b(); return c(); }`
#[must_use]
pub fn expand_arrow_sequences(code: &str) -> (String, usize) {
    let bytes = code.as_bytes();
    let mut result = String::with_capacity(code.len());
    let mut count = 0usize;

    let mut cursor = 0usize;
    let mut scan = 0usize;
    while let Some(arrow) = find_arrow_outside_syntax(code, scan) {
        let open = skip_ws(bytes, arrow + 2);
        if bytes.get(open) != Some(&b'(') {
            scan = arrow + 2;
            continue;
        }
        let Some(close) = find_matching_paren(code, open) else {
            scan = arrow + 2;
            continue;
        };

        let after_ws = skip_ws(bytes, close + 1);
        if !is_arrow_body_delimiter(bytes, after_ws) {
            scan = arrow + 2;
            continue;
        }

        let body = &code[open + 1..close];
        let Some(parts) = split_top_level_commas(body) else {
            scan = arrow + 2;
            continue;
        };
        if parts.len() < 3 {
            scan = arrow + 2;
            continue;
        }

        // Skip if any part looks complex outside string/comment/template syntax.
        if parts
            .iter()
            .any(|p| find_arrow_outside_syntax(p, 0).is_some() || contains_block_open_outside_syntax(p))
        {
            scan = arrow + 2;
            continue;
        }

        let (trailing, end) = match bytes.get(after_ws) {
            Some(b';' | b',') => (&code[after_ws..after_ws + 1], after_ws + 1),
            _ => ("", close + 1),
        };

        result.push_str(&code[cursor..arrow]);
        count += 1;
        result.push_str("=> {\n");
        for (i, part) in parts.iter().enumerate() {
            let trimmed = part.trim();
            if i == parts.len() - 1 {
                result.push_str(&format!("\t\treturn {trimmed};\n"));
            } else {
                result.push_str(&format!("\t\t{trimmed};\n"));
            }
        }
        result.push_str("\t}");
        result.push_str(trailing);

        cursor = end;
        scan = end;
    }

    result.push_str(&code[cursor..]);
    (result, count)
}

fn skip_ws(bytes: &[u8], mut i: usize) -> usize {
    while matches!(bytes.get(i), Some(b' ' | b'\n' | b'\r' | b'\t')) {
        i += 1;
    }
    i
}

fn skip_quoted(bytes: &[u8], mut i: usize, quote: u8) -> usize {
    i += 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i = (i + 2).min(bytes.len()),
            b if b == quote => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

fn skip_template(bytes: &[u8], mut i: usize) -> usize {
    i += 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i = (i + 2).min(bytes.len()),
            b'$' if bytes.get(i + 1) == Some(&b'{') => i = skip_template_expr(bytes, i + 2),
            b'`' => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

fn skip_template_expr(bytes: &[u8], mut i: usize) -> usize {
    let mut depth = 1i32;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' | b'"' => i = skip_quoted(bytes, i, bytes[i]),
            b'`' => i = skip_template(bytes, i),
            b'/' if bytes.get(i + 1) == Some(&b'/') => i = skip_line_comment(bytes, i),
            b'/' if bytes.get(i + 1) == Some(&b'*') => i = skip_block_comment(bytes, i),
            b'/' if is_regex_literal_start(bytes, i) => i = skip_regex_literal(bytes, i),
            b'{' => {
                depth += 1;
                i += 1;
            }
            b'}' => {
                depth -= 1;
                i += 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => i += 1,
        }
    }
    bytes.len()
}

fn skip_line_comment(bytes: &[u8], mut i: usize) -> usize {
    i += 2;
    while i < bytes.len() && !matches!(bytes[i], b'\n' | b'\r') {
        i += 1;
    }
    i
}

fn skip_block_comment(bytes: &[u8], mut i: usize) -> usize {
    i += 2;
    while i + 1 < bytes.len() {
        if bytes[i] == b'*' && bytes[i + 1] == b'/' {
            return i + 2;
        }
        i += 1;
    }
    bytes.len()
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

fn regex_prefix_keyword_before(bytes: &[u8], mut i: usize) -> bool {
    while i > 0 && matches!(bytes.get(i - 1), Some(b' ' | b'\n' | b'\r' | b'\t')) {
        i -= 1;
    }
    if i == 0 || !is_ident_continue(bytes[i - 1]) {
        return false;
    }

    let end = i;
    while i > 0 && is_ident_continue(bytes[i - 1]) {
        i -= 1;
    }
    matches!(
        std::str::from_utf8(&bytes[i..end]).ok(),
        Some("return" | "throw" | "case" | "delete" | "void" | "typeof" | "yield" | "await" | "in" | "instanceof")
    )
}

fn is_regex_literal_start(bytes: &[u8], i: usize) -> bool {
    if bytes.get(i) != Some(&b'/') || matches!(bytes.get(i + 1), Some(b'/' | b'*')) {
        return false;
    }
    if regex_prefix_keyword_before(bytes, i) {
        return true;
    }
    let mut prev = i;
    while prev > 0 && matches!(bytes.get(prev - 1), Some(b' ' | b'\n' | b'\r' | b'\t')) {
        prev -= 1;
    }
    prev == 0
        || matches!(
            bytes[prev - 1],
            b'(' | b'['
                | b'{'
                | b'='
                | b':'
                | b','
                | b';'
                | b'!'
                | b'?'
                | b'&'
                | b'|'
                | b'+'
                | b'-'
                | b'*'
                | b'%'
                | b'~'
                | b'^'
                | b'<'
                | b'>'
        )
}

fn skip_regex_literal(bytes: &[u8], mut i: usize) -> usize {
    let start = i;
    let mut in_class = false;
    i += 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i = (i + 2).min(bytes.len()),
            b'[' if !in_class => {
                in_class = true;
                i += 1;
            }
            b']' if in_class => {
                in_class = false;
                i += 1;
            }
            b'/' if !in_class => {
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
                    i += 1;
                }
                return i;
            }
            b'\n' | b'\r' => return start + 1,
            _ => i += 1,
        }
    }
    start + 1
}

fn is_arrow_body_delimiter(bytes: &[u8], i: usize) -> bool {
    i >= bytes.len() || matches!(bytes[i], b';' | b',' | b')' | b']' | b'}')
}

fn find_arrow_outside_syntax(s: &str, start: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = start;
    while i + 1 < bytes.len() {
        match bytes[i] {
            b'\'' | b'"' => i = skip_quoted(bytes, i, bytes[i]),
            b'`' => i = skip_template(bytes, i),
            b'/' if bytes.get(i + 1) == Some(&b'/') => i = skip_line_comment(bytes, i),
            b'/' if bytes.get(i + 1) == Some(&b'*') => i = skip_block_comment(bytes, i),
            b'/' if is_regex_literal_start(bytes, i) => i = skip_regex_literal(bytes, i),
            b'=' if bytes[i + 1] == b'>' => return Some(i),
            _ => i += 1,
        }
    }
    None
}

fn contains_block_open_outside_syntax(s: &str) -> bool {
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' | b'"' => i = skip_quoted(bytes, i, bytes[i]),
            b'`' => i = skip_template(bytes, i),
            b'/' if bytes.get(i + 1) == Some(&b'/') => i = skip_line_comment(bytes, i),
            b'/' if bytes.get(i + 1) == Some(&b'*') => i = skip_block_comment(bytes, i),
            b'/' if is_regex_literal_start(bytes, i) => i = skip_regex_literal(bytes, i),
            b'{' => return true,
            _ => i += 1,
        }
    }
    false
}

fn find_matching_paren(s: &str, open: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' | b'"' => i = skip_quoted(bytes, i, bytes[i]),
            b'`' => i = skip_template(bytes, i),
            b'/' if bytes.get(i + 1) == Some(&b'/') => i = skip_line_comment(bytes, i),
            b'/' if bytes.get(i + 1) == Some(&b'*') => i = skip_block_comment(bytes, i),
            b'/' if is_regex_literal_start(bytes, i) => i = skip_regex_literal(bytes, i),
            b'(' => {
                depth += 1;
                i += 1;
            }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
                if depth < 0 {
                    return None;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    None
}

/// Split a string on top-level commas (respecting nesting and string syntax).
fn split_top_level_commas(s: &str) -> Option<Vec<&str>> {
    let mut parts = Vec::new();
    let mut paren_depth = 0i32;
    let mut bracket_depth = 0i32;
    let mut brace_depth = 0i32;
    let mut last = 0;
    let bytes = s.as_bytes();
    let mut i = 0usize;

    while i < bytes.len() {
        match bytes[i] {
            b'\'' | b'"' => i = skip_quoted(bytes, i, bytes[i]),
            b'`' => i = skip_template(bytes, i),
            b'/' if bytes.get(i + 1) == Some(&b'/') => i = skip_line_comment(bytes, i),
            b'/' if bytes.get(i + 1) == Some(&b'*') => i = skip_block_comment(bytes, i),
            b'/' if is_regex_literal_start(bytes, i) => i = skip_regex_literal(bytes, i),
            b'(' => {
                paren_depth += 1;
                i += 1;
            }
            b'[' => {
                bracket_depth += 1;
                i += 1;
            }
            b'{' => {
                brace_depth += 1;
                i += 1;
            }
            b')' => {
                paren_depth -= 1;
                if paren_depth < 0 {
                    return None;
                }
                i += 1;
            }
            b']' => {
                bracket_depth -= 1;
                if bracket_depth < 0 {
                    return None;
                }
                i += 1;
            }
            b'}' => {
                brace_depth -= 1;
                if brace_depth < 0 {
                    return None;
                }
                i += 1;
            }
            b',' if paren_depth == 0 && bracket_depth == 0 && brace_depth == 0 => {
                parts.push(&s[last..i]);
                last = i + 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    if paren_depth != 0 || bracket_depth != 0 || brace_depth != 0 {
        return None;
    }
    parts.push(&s[last..]);
    Some(parts)
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

    #[test]
    fn expands_arrow_sequence_with_string_commas_safely() {
        let code = r#"const f = () => ("a,b", work(), done());"#;
        let (out, count) = expand_arrow_sequences(code);
        assert_eq!(count, 1);
        assert!(out.contains(r#""a,b";"#), "string literal must stay intact: {out}");
        assert!(out.contains("work();"), "Should split second part: {out}");
        assert!(out.contains("return done();"), "Should return final part: {out}");
    }

    #[test]
    fn ignores_arrow_text_inside_string() {
        let code = r#"const s = "=> (a(), b(), c())";"#;
        let (out, count) = expand_arrow_sequences(code);
        assert_eq!(count, 0);
        assert_eq!(out, code);
    }

    #[test]
    fn ignores_expression_continuation_after_parenthesized_sequence() {
        let code = "const f = () => (a(), b(), c()) + d;";
        let (out, count) = expand_arrow_sequences(code);
        assert_eq!(count, 0);
        assert_eq!(out, code);
    }

    #[test]
    fn ignores_arrow_text_inside_regex_literal() {
        let code = r#"const r = /=> (a(), b(), c())/;"#;
        let (out, count) = expand_arrow_sequences(code);
        assert_eq!(count, 0);
        assert_eq!(out, code);
    }

    #[test]
    fn expands_arrow_sequence_with_regex_commas_safely() {
        let code = r#"const f = () => (/a,b/.test(x), work(), done());"#;
        let (out, count) = expand_arrow_sequences(code);
        assert_eq!(count, 1);
        assert!(
            out.contains(r#"/a,b/.test(x);"#),
            "regex literal must stay intact: {out}"
        );
        assert!(out.contains("work();"), "Should split second part: {out}");
        assert!(out.contains("return done();"), "Should return final part: {out}");
    }

    #[test]
    fn ignores_arrow_text_inside_nested_template_literal() {
        let code = "const s = `${`=> (a(), b(), c());`}`;";
        let (out, count) = expand_arrow_sequences(code);
        assert_eq!(count, 0);
        assert_eq!(out, code);
    }
}
