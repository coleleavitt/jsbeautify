//! Output layout applied to codegen text: indentation, `eol`, the final
//! newline, preserved blank lines, function-paren spacing, `jslint_happy`
//! case alignment, brace placement, and js-beautify's line breaks for
//! one-line braces and `case` bodies.
//!
//! The text is re-parsed so that lines starting inside template literals,
//! multi-line strings or JSX text are copied verbatim; every other change
//! touches only whitespace between tokens.

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    BlockStatement, ClassBody, CommentKind, DoWhileStatement, ExportNamedDeclaration, Function, FunctionBody,
    IfStatement, ImportDeclaration, ImportDeclarationSpecifier, JSXText, MethodDefinition, ObjectExpression,
    ObjectPattern, ObjectProperty, PropertyKind, Statement, StringLiteral, SwitchCase, SwitchStatement,
    TemplateElement, TryStatement, VariableDeclaration,
};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType, Span};
use oxc_syntax::scope::ScopeFlags;

use crate::options::{BraceStyle, Options};

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone)]
pub struct Layout {
    indent: String,
    base: String,
    eol: &'static str,
    end_with_newline: bool,
    space_after_anon_function: bool,
    space_after_named_function: bool,
    dedent_switch_cases: bool,
    brace_style: BraceStyle,
}

impl Layout {
    /// Resolves the input-dependent settings (`eol` auto-detection and the
    /// base indent taken from the input's leading whitespace).
    #[must_use]
    pub fn new(options: &Options, input: &str) -> Self {
        let indent = if options.indent_with_tabs {
            "\t".to_string()
        } else {
            " ".repeat(options.indent_size)
        };
        let base = if options.indent_level > 0 {
            indent.repeat(options.indent_level)
        } else {
            input[..input.len() - input.trim_start_matches([' ', '\t']).len()].to_string()
        };
        Self {
            indent,
            base,
            eol: options.eol.resolve(input),
            end_with_newline: options.end_with_newline,
            space_after_anon_function: options.space_after_anon_function || options.jslint_happy,
            space_after_named_function: options.space_after_named_function,
            dedent_switch_cases: options.jslint_happy,
            brace_style: options.brace_style,
        }
    }

    /// Lays out codegen output (tab-indented, `\n`-separated). `blank_lines`
    /// lists `(line start offset, count)` pairs of empty lines to insert.
    #[must_use]
    pub fn apply(&self, code: &str, source_type: SourceType, blank_lines: &[(usize, usize)]) -> String {
        let allocator = Allocator::default();
        let parsed = Parser::new(&allocator, code, source_type).parse();
        let mut scan = Scan {
            code,
            layout: self,
            verbatim: Vec::new(),
            ranges: Vec::new(),
            edits: Vec::new(),
            blank_lines: blank_lines.to_vec(),
            comment_ends: parsed.program.comments.iter().map(|c| c.span.end).collect(),
            do_body: None,
        };
        scan.visit_program(&parsed.program);
        let mut javadoc: Vec<(usize, usize)> = parsed
            .program
            .comments
            .iter()
            .filter(|c| c.kind == CommentKind::MultiLineBlock)
            .map(|c| (c.span.start as usize, c.span.end as usize))
            .filter(|&(start, end)| {
                let text = &code[start..end];
                text.lines().skip(1).all(|line| line.trim_start().starts_with('*'))
            })
            .collect();
        let Scan {
            mut verbatim,
            mut ranges,
            mut edits,
            mut blank_lines,
            ..
        } = scan;
        verbatim.sort_unstable();
        ranges.sort_unstable();
        javadoc.sort_unstable();
        edits.sort_by_key(|&(at, _)| at);
        blank_lines.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
        blank_lines.dedup_by_key(|&mut (at, _)| at);
        Writer {
            layout: self,
            code,
            verbatim: &verbatim,
            javadoc: &javadoc,
            edits: &edits,
            blank_lines: &blank_lines,
            depth: Depth {
                ranges: &ranges,
                next: 0,
                open: Vec::new(),
            },
        }
        .write()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Edit {
    /// Make the space before the `(` at this offset present or absent.
    Space(bool),
    /// Start a new line at this offset, indented this many levels beyond
    /// the level at the offset.
    Break(isize),
}

struct Scan<'s> {
    code: &'s str,
    layout: &'s Layout,
    /// `(start, end)`: a line starting in `start + 1..=end` is content.
    verbatim: Vec<(usize, usize)>,
    /// `(open, close, delta)`: text strictly between `open` and `close` is
    /// indented `delta` levels beyond codegen's indentation.
    ranges: Vec<(usize, usize, isize)>,
    edits: Vec<(usize, Edit)>,
    blank_lines: Vec<(usize, usize)>,
    comment_ends: Vec<u32>,
    do_body: Option<u32>,
}

impl Scan<'_> {
    fn skip_ws(&self, mut at: usize) -> usize {
        let bytes = self.code.as_bytes();
        while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        }
        at
    }

    fn spacing(&mut self, paren: usize, space: bool) {
        if self.code.as_bytes().get(paren) == Some(&b'(') {
            self.edits.push((paren, Edit::Space(space)));
        }
    }

    fn open_brace(&mut self, at: u32) {
        if self.layout.brace_style == BraceStyle::Expand {
            self.edits.push((at as usize, Edit::Break(0)));
        }
    }

    fn continuation(&mut self, at: usize) {
        if self.layout.brace_style != BraceStyle::Collapse {
            self.edits.push((at, Edit::Break(0)));
        }
    }

    /// js-beautify puts the items of every non-empty `{ … }` list on their
    /// own lines; codegen keeps some on one line.
    fn expand_braces(&mut self, items: Option<(Span, Span)>) {
        let Some((first, last)) = items else {
            return;
        };
        let (first, last) = (first.start as usize, last.end as usize);
        let before = self.code[..first].trim_end_matches([' ', '\t']);
        let close = self.skip_ws(last);
        let close = if self.code.as_bytes().get(close) == Some(&b',') {
            self.skip_ws(close + 1)
        } else {
            close
        };
        if !before.ends_with('{') || self.code.as_bytes().get(close) != Some(&b'}') {
            return;
        }
        let open = before.len() - 1;
        self.ranges.push((open, close, 1));
        self.edits.push((first, Edit::Break(0)));
        self.edits.push((close, Edit::Break(0)));
    }

    fn line_start(&self, at: usize) -> usize {
        self.code[..at].rfind(['\n', '\r']).map_or(0, |i| i + 1)
    }
}

impl<'a> Visit<'a> for Scan<'_> {
    fn visit_template_element(&mut self, it: &TemplateElement<'a>) {
        self.verbatim.push((it.span.start as usize, it.span.end as usize));
    }

    fn visit_string_literal(&mut self, it: &StringLiteral<'a>) {
        self.verbatim.push((it.span.start as usize, it.span.end as usize - 1));
    }

    fn visit_jsx_text(&mut self, it: &JSXText<'a>) {
        self.verbatim.push((it.span.start as usize, it.span.end as usize));
    }

    fn visit_statement(&mut self, it: &Statement<'a>) {
        if let Statement::FunctionDeclaration(function) = it {
            let start = function.span.start as usize;
            let line_start = self.line_start(start);
            let previous = self.code[..line_start].trim_end();
            let after_statement =
                previous.ends_with(['}', ';']) && self.comment_ends.binary_search(&(previous.len() as u32)).is_err();
            if self.code[start..].starts_with("function")
                && after_statement
                && self.code[line_start..start].bytes().all(|b| b == b' ' || b == b'\t')
            {
                self.blank_lines.push((line_start, 1));
            }
        }
        walk::walk_statement(self, it);
    }

    fn visit_switch_statement(&mut self, it: &SwitchStatement<'a>) {
        let discriminant_end = it.discriminant.span().end as usize;
        if let Some(open) = self.code[discriminant_end..].find('{').map(|i| i + discriminant_end) {
            if self.layout.dedent_switch_cases {
                self.ranges.push((open, it.span.end as usize - 1, -1));
            }
            self.open_brace(open as u32);
        }
        walk::walk_switch_statement(self, it);
    }

    fn visit_switch_case(&mut self, it: &SwitchCase<'a>) {
        if let Some(first) = it.consequent.first()
            && !matches!(first, Statement::BlockStatement(_))
        {
            let start = first.span().start as usize;
            if self.line_start(start) <= it.span.start as usize {
                self.edits.push((start, Edit::Break(1)));
            }
        }
        walk::walk_switch_case(self, it);
    }

    fn visit_function(&mut self, it: &Function<'a>, flags: ScopeFlags) {
        let start = it.span.start as usize;
        let header = &self.code[start..];
        if header.starts_with("function") || header.starts_with("async") {
            match &it.id {
                Some(id) => {
                    if self.layout.space_after_named_function {
                        self.spacing(id.span.end as usize, true);
                    }
                }
                None => {
                    if let Some(paren) = header.find('(') {
                        self.spacing(start + paren, self.layout.space_after_anon_function);
                    }
                }
            }
        }
        walk::walk_function(self, it, flags);
    }

    fn visit_method_definition(&mut self, it: &MethodDefinition<'a>) {
        if self.layout.space_after_named_function && !it.computed {
            self.spacing(it.key.span().end as usize, true);
        }
        walk::walk_method_definition(self, it);
    }

    fn visit_object_property(&mut self, it: &ObjectProperty<'a>) {
        if self.layout.space_after_named_function && it.method && !it.computed && it.kind == PropertyKind::Init {
            self.spacing(it.key.span().end as usize, true);
        }
        walk::walk_object_property(self, it);
    }

    fn visit_object_expression(&mut self, it: &ObjectExpression<'a>) {
        self.expand_braces(
            it.properties
                .first()
                .zip(it.properties.last())
                .map(|(f, l)| (f.span(), l.span())),
        );
        walk::walk_object_expression(self, it);
    }

    fn visit_object_pattern(&mut self, it: &ObjectPattern<'a>) {
        let first = it.properties.first().map(GetSpan::span);
        let last = it
            .rest
            .as_ref()
            .map(|rest| rest.span)
            .or_else(|| it.properties.last().map(GetSpan::span));
        self.expand_braces(first.or(last).zip(last));
        walk::walk_object_pattern(self, it);
    }

    fn visit_import_declaration(&mut self, it: &ImportDeclaration<'a>) {
        let named: Vec<Span> = it
            .specifiers
            .iter()
            .flatten()
            .filter(|s| matches!(s, ImportDeclarationSpecifier::ImportSpecifier(_)))
            .map(GetSpan::span)
            .collect();
        self.expand_braces(named.first().copied().zip(named.last().copied()));
        walk::walk_import_declaration(self, it);
    }

    fn visit_export_named_declaration(&mut self, it: &ExportNamedDeclaration<'a>) {
        self.expand_braces(
            it.specifiers
                .first()
                .zip(it.specifiers.last())
                .map(|(f, l)| (f.span, l.span)),
        );
        walk::walk_export_named_declaration(self, it);
    }

    /// js-beautify breaks a declaration list after every declarator with an
    /// initializer and indents the continuation.
    fn visit_variable_declaration(&mut self, it: &VariableDeclaration<'a>) {
        let in_for_head = self.code[..it.span.start as usize].trim_end().ends_with('(');
        if !in_for_head {
            let breaks: Vec<usize> = it
                .declarations
                .windows(2)
                .filter(|pair| pair[0].init.is_some())
                .map(|pair| pair[1].span.start as usize)
                .collect();
            if !breaks.is_empty() {
                self.ranges
                    .push((it.declarations[0].span.start as usize, it.span.end as usize, 1));
                self.edits.extend(breaks.into_iter().map(|at| (at, Edit::Break(0))));
            }
        }
        walk::walk_variable_declaration(self, it);
    }

    fn visit_do_while_statement(&mut self, it: &DoWhileStatement<'a>) {
        if let Statement::BlockStatement(body) = &it.body {
            self.do_body = Some(body.span.start);
        }
        walk::walk_do_while_statement(self, it);
    }

    fn visit_block_statement(&mut self, it: &BlockStatement<'a>) {
        if self.do_body != Some(it.span.start) {
            self.open_brace(it.span.start);
        }
        walk::walk_block_statement(self, it);
    }

    fn visit_function_body(&mut self, it: &FunctionBody<'a>) {
        if self.code.as_bytes().get(it.span.start as usize) == Some(&b'{') {
            self.open_brace(it.span.start);
        }
        walk::walk_function_body(self, it);
    }

    fn visit_class_body(&mut self, it: &ClassBody<'a>) {
        self.open_brace(it.span.start);
        walk::walk_class_body(self, it);
    }

    fn visit_if_statement(&mut self, it: &IfStatement<'a>) {
        if it.alternate.is_some() && matches!(it.consequent, Statement::BlockStatement(_)) {
            let at = self.skip_ws(it.consequent.span().end as usize);
            self.continuation(at);
        }
        walk::walk_if_statement(self, it);
    }

    fn visit_try_statement(&mut self, it: &TryStatement<'a>) {
        if let Some(handler) = &it.handler {
            self.continuation(handler.span.start as usize);
        }
        let previous_end = it
            .handler
            .as_ref()
            .map_or(it.block.span.end, |handler| handler.span.end);
        if it.finalizer.is_some() {
            let at = self.skip_ws(previous_end as usize);
            self.continuation(at);
        }
        walk::walk_try_statement(self, it);
    }
}

/// Indentation adjustment at increasing offsets.
struct Depth<'r> {
    ranges: &'r [(usize, usize, isize)],
    next: usize,
    open: Vec<(usize, isize)>,
}

impl Depth<'_> {
    fn at(&mut self, offset: usize) -> isize {
        while let Some(&(open, close, delta)) = self.ranges.get(self.next)
            && open < offset
        {
            self.open.push((close, delta));
            self.next += 1;
        }
        self.open.retain(|&(close, _)| close > offset);
        self.open.iter().map(|&(_, delta)| delta).sum()
    }
}

struct Writer<'w> {
    layout: &'w Layout,
    code: &'w str,
    verbatim: &'w [(usize, usize)],
    javadoc: &'w [(usize, usize)],
    edits: &'w [(usize, Edit)],
    blank_lines: &'w [(usize, usize)],
    depth: Depth<'w>,
}

impl Writer<'_> {
    fn push_indent(&self, out: &mut String, level: isize) {
        out.push_str(&self.layout.base);
        for _ in 0..level.max(0) {
            out.push_str(&self.layout.indent);
        }
    }

    fn write(mut self) -> String {
        let eol = self.layout.eol;
        let code = self.code;
        let mut out = String::with_capacity(code.len() + code.len() / 4);
        let (mut verbatim, mut javadoc, mut edit, mut blank) = (0, 0, 0, 0);
        let mut start = 0;
        while start <= code.len() {
            let (end, next) = code[start..]
                .find(['\n', '\r'])
                .map_or((code.len(), code.len() + 1), |i| {
                    let end = start + i;
                    (end, end + if code[end..].starts_with("\r\n") { 2 } else { 1 })
                });
            if start > 0 {
                out.push_str(eol);
            }
            let line = &code[start..end];
            while self.verbatim.get(verbatim).is_some_and(|&(_, e)| e < start) {
                verbatim += 1;
            }
            if self.verbatim.get(verbatim).is_some_and(|&(s, _)| s < start) {
                out.push_str(line);
                start = next;
                continue;
            }

            while self.blank_lines.get(blank).is_some_and(|&(at, _)| at < start) {
                blank += 1;
            }
            if let Some(&(at, count)) = self.blank_lines.get(blank)
                && at == start
            {
                for _ in 0..count {
                    out.push_str(eol);
                }
            }

            let content = line.trim_start_matches('\t');
            if content.trim().is_empty() {
                start = next;
                continue;
            }
            let tabs = (line.len() - content.len()).cast_signed();
            let content_start = end - content.len();
            let level = tabs + self.depth.at(content_start);
            self.push_indent(&mut out, level);
            while self.javadoc.get(javadoc).is_some_and(|&(_, e)| e < content_start) {
                javadoc += 1;
            }
            if self.javadoc.get(javadoc).is_some_and(|&(s, _)| s < content_start) {
                out.push(' ');
            }
            let mut line_has_content = false;
            let mut cursor = content_start;
            while self.edits.get(edit).is_some_and(|&(at, _)| at < content_start) {
                edit += 1;
            }
            while let Some(&(at, kind)) = self.edits.get(edit)
                && at < end
            {
                line_has_content |= at > cursor;
                out.push_str(&code[cursor..at]);
                cursor = at;
                match kind {
                    Edit::Space(true) => {
                        if !out.ends_with(' ') {
                            out.push(' ');
                        }
                    }
                    Edit::Space(false) => out.truncate(out.trim_end_matches(' ').len()),
                    Edit::Break(extra) => {
                        if line_has_content {
                            out.truncate(out.trim_end_matches(' ').len());
                            out.push_str(eol);
                            let level = tabs + self.depth.at(at) + extra;
                            self.push_indent(&mut out, level);
                            line_has_content = false;
                        }
                    }
                }
                edit += 1;
            }
            out.push_str(&code[cursor..end]);
            start = next;
        }
        out.truncate(out.trim_end_matches(['\n', '\r', ' ', '\t']).len());
        if self.layout.end_with_newline {
            out.push_str(eol);
        }
        out
    }
}
