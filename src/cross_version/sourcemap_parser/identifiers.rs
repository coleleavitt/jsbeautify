use oxc_allocator::Allocator;
use oxc_ast::ast::{BindingIdentifier, IdentifierReference};
use oxc_ast_visit::Visit;
use oxc_parser::Parser;
use oxc_span::SourceType;

use super::SourcemapParser;

pub(super) struct IdentifierCollector<'a> {
    #[allow(dead_code)]
    source: &'a str,
    line_starts: Vec<u32>,
    identifiers: Vec<(u32, u32, String)>,
}

impl<'a> IdentifierCollector<'a> {
    fn new(source: &'a str) -> Self {
        let mut line_starts = vec![0u32];
        for (i, ch) in source.char_indices() {
            if ch == '\n' {
                line_starts.push((i + 1) as u32);
            }
        }
        Self {
            source,
            line_starts,
            identifiers: Vec::new(),
        }
    }

    // Sourcemap columns are UTF-16 code units, not byte offsets.
    // Convert byte offset to (1-based line, 0-based UTF-16 column).
    fn offset_to_line_col(&self, offset: u32) -> (u32, u32) {
        let line_idx = self.line_starts.partition_point(|&start| start <= offset);
        let line = line_idx as u32;
        let line_start = self.line_starts.get(line_idx.saturating_sub(1)).copied().unwrap_or(0) as usize;
        let offset = offset as usize;

        let line_slice = &self.source.as_bytes()[line_start..offset];
        let mut utf16_col = 0u32;
        let mut i = 0;
        while i < line_slice.len() {
            let b = line_slice[i];
            let (char_bytes, utf16_units) = if b < 0x80 {
                (1, 1)
            } else if b < 0xE0 {
                (2, 1)
            } else if b < 0xF0 {
                (3, 1)
            } else {
                // 4-byte UTF-8 = surrogate pair = 2 UTF-16 code units
                (4, 2)
            };
            utf16_col += utf16_units;
            i += char_bytes;
        }

        (line, utf16_col)
    }

    fn add_identifier(&mut self, name: &str, start: u32) {
        let (line, col) = self.offset_to_line_col(start);
        self.identifiers.push((line, col, name.to_string()));
    }
}

impl<'a> Visit<'a> for IdentifierCollector<'a> {
    fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
        self.add_identifier(it.name.as_str(), it.span.start);
        oxc_ast_visit::walk::walk_identifier_reference(self, it);
    }

    fn visit_binding_identifier(&mut self, it: &BindingIdentifier<'a>) {
        self.add_identifier(it.name.as_str(), it.span.start);
        oxc_ast_visit::walk::walk_binding_identifier(self, it);
    }
}

impl SourcemapParser {
    pub(super) fn extract_identifiers_ast(&self, source: &str) -> Vec<(u32, u32, String)> {
        let allocator = Allocator::default();
        let source_type = SourceType::mjs();
        let parser = Parser::new(&allocator, source, source_type);
        let result = parser.parse();

        if result.panicked || !result.diagnostics.is_empty() {
            eprintln!("[SOURCEMAP] AST parse failed, using regex fallback");
            return self.extract_identifiers_fallback(source);
        }

        let mut collector = IdentifierCollector::new(source);
        collector.visit_program(&result.program);
        collector.identifiers
    }

    fn extract_identifiers_fallback(&self, source: &str) -> Vec<(u32, u32, String)> {
        let mut result = Vec::new();
        let mut in_string = false;
        let mut string_char = ' ';
        let mut prev_char = ' ';
        let mut current_word = String::new();
        let mut word_start_col = 0u32;
        let mut line = 1u32;
        let mut col = 0u32;

        for ch in source.chars() {
            if ch == '\n' {
                if !current_word.is_empty() && self.is_identifier(&current_word) {
                    result.push((line, word_start_col, current_word.clone()));
                }
                current_word.clear();
                line += 1;
                col = 0;
                prev_char = ch;
                continue;
            }

            if !in_string && (ch == '"' || ch == '\'' || ch == '`') {
                if !current_word.is_empty() && self.is_identifier(&current_word) {
                    result.push((line, word_start_col, current_word.clone()));
                }
                current_word.clear();
                in_string = true;
                string_char = ch;
                prev_char = ch;
                col += 1;
                continue;
            }

            if in_string {
                if ch == string_char && prev_char != '\\' {
                    in_string = false;
                }
                prev_char = ch;
                col += 1;
                continue;
            }

            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '$' {
                if current_word.is_empty() {
                    word_start_col = col;
                }
                current_word.push(ch);
            } else {
                if !current_word.is_empty() && self.is_identifier(&current_word) {
                    result.push((line, word_start_col, current_word.clone()));
                }
                current_word.clear();
            }

            prev_char = ch;
            col += 1;
        }

        if !current_word.is_empty() && self.is_identifier(&current_word) {
            result.push((line, word_start_col, current_word));
        }

        result
    }
}
