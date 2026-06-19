use super::syntax_scan::{
    contains_block_open_outside_syntax, find_arrow_outside_syntax, find_matching_paren, is_arrow_body_delimiter,
    skip_ws, split_top_level_commas,
};
use std::fmt::Write as _;

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
            Some(b';' | b',') => (&code[after_ws..=after_ws], after_ws + 1),
            _ => ("", close + 1),
        };

        result.push_str(&code[cursor..arrow]);
        count += 1;
        result.push_str("=> {\n");
        for (i, part) in parts.iter().enumerate() {
            let trimmed = part.trim();
            if i == parts.len() - 1 {
                let _ = writeln!(result, "\t\treturn {trimmed};");
            } else {
                let _ = writeln!(result, "\t\t{trimmed};");
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
