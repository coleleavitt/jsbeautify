mod skip;

pub(super) use skip::skip_ws;
use skip::{
    is_regex_literal_start, skip_block_comment, skip_line_comment, skip_quoted, skip_regex_literal, skip_template,
};

pub(super) const fn is_arrow_body_delimiter(bytes: &[u8], i: usize) -> bool {
    i >= bytes.len() || matches!(bytes[i], b';' | b',' | b')' | b']' | b'}')
}

/// Offsets in a text outside strings, comments, templates and regex
/// literals, found by one scan so that each lookup is cheap however deeply
/// the text nests.
pub(super) struct SyntaxIndex {
    /// Offsets of `=>`.
    pub(super) arrows: Vec<usize>,
    /// Sorted offsets of `{`.
    block_opens: Vec<usize>,
    /// `(open, close)` of each matched paren pair, sorted by `open`.
    parens: Vec<(usize, usize)>,
}

impl SyntaxIndex {
    pub(super) fn new(s: &str) -> Self {
        let bytes = s.as_bytes();
        let mut index = Self {
            arrows: Vec::new(),
            block_opens: Vec::new(),
            parens: Vec::new(),
        };
        let mut open_parens = Vec::new();
        let mut i = 0usize;
        while i < bytes.len() {
            match bytes[i] {
                b'\'' | b'"' => i = skip_quoted(bytes, i, bytes[i]),
                b'`' => i = skip_template(bytes, i),
                b'/' if bytes.get(i + 1) == Some(&b'/') => i = skip_line_comment(bytes, i),
                b'/' if bytes.get(i + 1) == Some(&b'*') => i = skip_block_comment(bytes, i),
                b'/' if is_regex_literal_start(bytes, i) => i = skip_regex_literal(bytes, i),
                b'=' if bytes.get(i + 1) == Some(&b'>') => {
                    index.arrows.push(i);
                    i += 2;
                }
                b'{' => {
                    index.block_opens.push(i);
                    i += 1;
                }
                b'(' => {
                    open_parens.push(i);
                    i += 1;
                }
                b')' => {
                    if let Some(open) = open_parens.pop() {
                        index.parens.push((open, i));
                    }
                    i += 1;
                }
                _ => i += 1,
            }
        }
        index.parens.sort_unstable();
        index
    }

    /// The `)` closing the `(` at `open`.
    pub(super) fn matching_paren(&self, open: usize) -> Option<usize> {
        self.parens
            .binary_search_by_key(&open, |&(start, _)| start)
            .ok()
            .map(|at| self.parens[at].1)
    }

    /// Whether `start..end` holds an `=>` or a `{`.
    pub(super) fn has_arrow_or_block_open(&self, start: usize, end: usize) -> bool {
        let within = |offsets: &[usize]| {
            offsets
                .get(offsets.partition_point(|&at| at < start))
                .is_some_and(|&at| at < end)
        };
        within(&self.arrows) || within(&self.block_opens)
    }
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
