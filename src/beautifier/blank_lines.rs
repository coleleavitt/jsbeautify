//! js-beautify's `preserve_newlines`: blank lines before statements, class
//! members, switch cases, object properties and closing braces, and before
//! the comments that precede them, are carried from the input into codegen
//! output.

use oxc_ast::ast::{
    BlockStatement, ClassBody, ClassElement, Comment, Directive, FunctionBody, ObjectPropertyKind, Program, Statement,
    SwitchCase,
};
use oxc_ast_visit::{Visit, walk};
use oxc_span::GetSpan;
use rustc_hash::FxHashMap;

/// A source token: an anchor node start or a comment, with the number of
/// empty lines to keep before it.
struct Item {
    start: usize,
    blanks: usize,
    /// A comment's text lines, trimmed; empty for the anchor.
    lines: Vec<String>,
}

/// Returns `(output line start, empty line count)` pairs sorted by offset.
/// `mappings` yields codegen source-map tokens as
/// `(dst line, dst column, src line, src column)`, columns in UTF-16 units.
pub fn blank_lines(
    source: &str,
    program: &Program<'_>,
    output: &str,
    mappings: impl Iterator<Item = (u32, u32, u32, u32)>,
    max: usize,
) -> Vec<(usize, usize)> {
    let mut anchors = Anchors {
        source,
        starts: Vec::new(),
    };
    anchors.visit_program(program);
    let mut starts = anchors.starts;
    starts.sort_unstable();
    starts.dedup();

    let groups: Vec<Vec<Item>> = starts
        .into_iter()
        .map(|start| preceding_items(source, &program.comments, start as usize, max))
        .filter(|items| items.iter().any(|item| item.blanks > 0))
        .collect();
    let output_lines = Lines::new(output);
    let mut result = Vec::new();
    let trailing = preceding_items(source, &program.comments, source.trim_end().len(), max);
    let last_line = (0..output_lines.starts.len())
        .rev()
        .find(|&l| !output_lines.line(output, l).trim().is_empty());
    if let Some(last_line) = last_line {
        match_comments(output, &output_lines, last_line + 1, &trailing[1..], &mut result);
    }
    if groups.is_empty() {
        return finish(result);
    }

    let source_lines = Lines::new(source);
    let mut wanted: FxHashMap<(u32, u32), Option<(u32, u32)>> = groups
        .iter()
        .map(|items| (source_lines.position(source, items[0].start), None))
        .collect();
    for (dst_line, dst_col, src_line, src_col) in mappings {
        if let Some(slot @ None) = wanted.get_mut(&(src_line, src_col)) {
            *slot = Some((dst_line, dst_col));
        }
    }

    for items in &groups {
        let Some(Some((dst_line, dst_col))) = wanted.get(&source_lines.position(source, items[0].start)) else {
            continue;
        };
        let line = *dst_line as usize;
        let Some(at) = output_lines.offset(output, line, *dst_col) else {
            continue;
        };
        if !output[output_lines.starts[line]..at]
            .bytes()
            .all(|b| b == b' ' || b == b'\t')
        {
            continue;
        }
        result.push((output_lines.starts[line], items[0].blanks));
        match_comments(output, &output_lines, line, &items[1..], &mut result);
    }
    finish(result)
}

fn finish(mut result: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    result.retain(|&(at, blanks)| at > 0 && blanks > 0);
    result.sort_unstable();
    result.dedup_by_key(|&mut (at, _)| at);
    result
}

/// Walks up from output `line` over the comments (nearest first) codegen
/// printed on their own lines above it, recording their blank lines.
fn match_comments(output: &str, lines: &Lines, mut line: usize, comments: &[Item], result: &mut Vec<(usize, usize)>) {
    for comment in comments {
        let Some(first) = line.checked_sub(comment.lines.len()) else {
            return;
        };
        let matches = (first..line)
            .zip(&comment.lines)
            .all(|(l, expected)| lines.line(output, l).trim() == expected);
        if !matches {
            return;
        }
        line = first;
        result.push((lines.starts[line], comment.blanks));
    }
}

/// The anchor at `start` followed by the comments directly before it,
/// nearest first.
/// The anchor at `start` followed by the comments directly before it,
/// nearest first.
fn preceding_items(source: &str, comments: &[Comment], start: usize, max: usize) -> Vec<Item> {
    let mut items = Vec::new();
    let mut item_start = start;
    let mut lines = Vec::new();
    loop {
        let gap_start = source[..item_start].trim_end().len();
        let newlines = split_lines(&source[gap_start..item_start]).count() - 1;
        let blanks = if gap_start == 0 || newlines < 2 {
            0
        } else if max == 0 {
            newlines - 1
        } else {
            newlines.min(max) - 1
        };
        items.push(Item {
            start: item_start,
            blanks,
            lines,
        });
        match comments.binary_search_by_key(&(gap_start as u32), |c| c.span.end) {
            Ok(i) if gap_start > 0 => {
                item_start = comments[i].span.start as usize;
                lines = split_lines(&source[item_start..gap_start])
                    .map(|line| line.trim().to_string())
                    .collect();
            }
            _ => return items,
        }
    }
}

struct Anchors<'s> {
    source: &'s str,
    starts: Vec<u32>,
}

impl Anchors<'_> {
    fn closing_brace(&mut self, end: u32) {
        if end > 0 && self.source.as_bytes().get(end as usize - 1) == Some(&b'}') {
            self.starts.push(end - 1);
        }
    }
}

impl<'a> Visit<'a> for Anchors<'_> {
    fn visit_statement(&mut self, it: &Statement<'a>) {
        self.starts.push(it.span().start);
        walk::walk_statement(self, it);
    }

    fn visit_directive(&mut self, it: &Directive<'a>) {
        self.starts.push(it.span.start);
        walk::walk_directive(self, it);
    }

    fn visit_class_element(&mut self, it: &ClassElement<'a>) {
        self.starts.push(it.span().start);
        walk::walk_class_element(self, it);
    }

    fn visit_switch_case(&mut self, it: &SwitchCase<'a>) {
        self.starts.push(it.span.start);
        walk::walk_switch_case(self, it);
    }

    fn visit_object_property_kind(&mut self, it: &ObjectPropertyKind<'a>) {
        self.starts.push(it.span().start);
        walk::walk_object_property_kind(self, it);
    }

    fn visit_block_statement(&mut self, it: &BlockStatement<'a>) {
        self.closing_brace(it.span.end);
        walk::walk_block_statement(self, it);
    }

    fn visit_function_body(&mut self, it: &FunctionBody<'a>) {
        self.closing_brace(it.span.end);
        walk::walk_function_body(self, it);
    }

    fn visit_class_body(&mut self, it: &ClassBody<'a>) {
        self.closing_brace(it.span.end);
        walk::walk_class_body(self, it);
    }
}

/// Splits on the line terminators the source map counts: `\r\n`, `\r`,
/// `\n`, U+2028 and U+2029.
fn split_lines(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = Some(text);
    std::iter::from_fn(move || {
        let text = rest?;
        let Some(i) = text.find(['\r', '\n', '\u{2028}', '\u{2029}']) else {
            rest = None;
            return Some(text);
        };
        let width = if text[i..].starts_with("\r\n") {
            2
        } else {
            text[i..].chars().next().map_or(1, char::len_utf8)
        };
        rest = Some(&text[i + width..]);
        Some(&text[..i])
    })
}

struct Lines {
    starts: Vec<usize>,
}

impl Lines {
    fn new(text: &str) -> Self {
        let mut starts = vec![0];
        let mut offset = 0;
        let mut pieces = split_lines(text).peekable();
        while let Some(line) = pieces.next() {
            if pieces.peek().is_none() {
                break;
            }
            offset += line.len();
            offset += if text[offset..].starts_with("\r\n") {
                2
            } else {
                text[offset..].chars().next().map_or(1, char::len_utf8)
            };
            starts.push(offset);
        }
        Self { starts }
    }

    fn line<'t>(&self, text: &'t str, line: usize) -> &'t str {
        let end = self.starts.get(line + 1).map_or(text.len(), |&next| next);
        split_lines(&text[self.starts[line]..end]).next().unwrap_or_default()
    }

    fn position(&self, text: &str, offset: usize) -> (u32, u32) {
        let line = self.starts.partition_point(|&s| s <= offset) - 1;
        let column: usize = text[self.starts[line]..offset].chars().map(char::len_utf16).sum();
        (line as u32, column as u32)
    }

    fn offset(&self, text: &str, line: usize, column: u32) -> Option<usize> {
        let start = *self.starts.get(line)?;
        let mut units = 0;
        for (i, c) in text[start..].char_indices() {
            if units == column as usize {
                return Some(start + i);
            }
            units += c.len_utf16();
        }
        (units == column as usize).then_some(text.len())
    }
}
