mod skip;

pub(super) use skip::skip_ws;
use skip::{
    is_regex_literal_start, skip_block_comment, skip_line_comment, skip_quoted, skip_regex_literal, skip_template,
};

pub(super) const fn is_arrow_body_delimiter(bytes: &[u8], i: usize) -> bool {
    i >= bytes.len() || matches!(bytes[i], b';' | b',' | b')' | b']' | b'}')
}

pub(super) fn find_arrow_outside_syntax(s: &str, start: usize) -> Option<usize> {
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

pub(super) fn contains_block_open_outside_syntax(s: &str) -> bool {
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

pub(super) fn find_matching_paren(s: &str, open: usize) -> Option<usize> {
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
pub(super) fn split_top_level_commas(s: &str) -> Option<Vec<&str>> {
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
