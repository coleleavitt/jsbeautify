use super::syntax_scan::{SyntaxIndex, is_arrow_body_delimiter, skip_ws, split_top_level_commas};
use std::fmt::Write as _;

// ─────────────────────────────────────────────────────────────────────────────
// 3. Arrow Sequence Expansion (post-codegen string replacement)
// ─────────────────────────────────────────────────────────────────────────────

/// Expands simple arrow sequences with 2+ top-level commas into block form.
/// `(H) => (a(), b(), c())` → `(H) => { a(); b(); return c(); }`
#[must_use]
pub fn expand_arrow_sequences(code: &str) -> (String, usize) {
    let bytes = code.as_bytes();
    let index = SyntaxIndex::new(code);
    let mut result = String::with_capacity(code.len());
    let mut count = 0usize;

    let mut cursor = 0usize;
    for &arrow in &index.arrows {
        if arrow < cursor {
            continue;
        }
        let open = skip_ws(bytes, arrow + 2);
        if bytes.get(open) != Some(&b'(') {
            continue;
        }
        let Some(close) = index.matching_paren(open) else {
            continue;
        };

        let after_ws = skip_ws(bytes, close + 1);
        if !is_arrow_body_delimiter(bytes, after_ws) {
            continue;
        }

        // Skip a body holding a nested arrow or a block. Checking this before
        // splitting keeps the work linear: the bodies that get split never
        // nest.
        if index.has_arrow_or_block_open(open + 1, close) {
            continue;
        }
        let body = &code[open + 1..close];
        let Some(parts) = split_top_level_commas(body) else {
            continue;
        };
        if parts.len() < 3 {
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
    }

    result.push_str(&code[cursor..]);
    (result, count)
}
