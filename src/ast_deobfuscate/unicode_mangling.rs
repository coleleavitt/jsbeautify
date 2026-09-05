use oxc_ast::ast::Expression;
use oxc_span::SPAN;
use oxc_traverse::{Traverse, TraverseCtx};

use crate::ast_deobfuscate::pass::flag_pass;
use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

const ZERO_WIDTH_CHARS: &[char] = &[
    '\u{200B}', // ZERO WIDTH SPACE
    '\u{200C}', // ZERO WIDTH NON-JOINER
    '\u{200D}', // ZERO WIDTH JOINER
    '\u{FEFF}', // ZERO WIDTH NO-BREAK SPACE
    '\u{2060}', // WORD JOINER
];

pub struct UnicodeNormalizer {
    changed: bool,
}

impl UnicodeNormalizer {
    fn contains_problematic_unicode(text: &str) -> bool {
        for ch in text.chars() {
            if ZERO_WIDTH_CHARS.contains(&ch) || Self::is_confusable(ch) {
                return true;
            }
        }
        false
    }

    const fn is_confusable(ch: char) -> bool {
        matches!(
            ch,
            '\u{0410}'..='\u{044F}' | // Cyrillic
            '\u{0391}'..='\u{03C9}'   // Greek
        )
    }

    fn normalize_string(text: &str) -> String {
        text.chars()
            .filter(|ch| !ZERO_WIDTH_CHARS.contains(ch))
            .map(Self::normalize_confusable)
            .collect()
    }

    const fn normalize_confusable(ch: char) -> char {
        match ch {
            '\u{0410}' | '\u{0391}' => 'A',
            '\u{0412}' | '\u{0392}' => 'B',
            '\u{0415}' | '\u{0395}' => 'E',
            '\u{041A}' | '\u{039A}' => 'K',
            '\u{041C}' | '\u{039C}' => 'M',
            '\u{041D}' | '\u{0397}' => 'H',
            '\u{041E}' | '\u{039F}' => 'O',
            '\u{0420}' | '\u{03A1}' => 'P',
            '\u{0421}' => 'C',
            '\u{0422}' | '\u{03A4}' => 'T',
            '\u{0425}' | '\u{03A7}' => 'X',
            '\u{0430}' => 'a',
            '\u{0435}' => 'e',
            '\u{043E}' => 'o',
            '\u{0440}' => 'p',
            '\u{0441}' => 'c',
            '\u{0445}' => 'x',
            '\u{0396}' => 'Z',
            '\u{0399}' => 'I',
            '\u{039D}' => 'N',
            '\u{03A5}' => 'Y',
            _ => ch,
        }
    }
}

flag_pass!(UnicodeNormalizer, changed, has_changed);

impl<'a> Traverse<'a, DeobfuscateState> for UnicodeNormalizer {
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        if let Expression::StringLiteral(lit) = expr {
            let text = lit.value.as_str();
            if Self::contains_problematic_unicode(text) {
                let normalized = Self::normalize_string(text);
                eprintln!("[AST] Normalizing unicode in string literal");
                self.changed = true;
                *expr = Expression::new_string_literal(
                    SPAN,
                    oxc_ast::ast::Str::from_str_in(&normalized, &ctx.ast),
                    None,
                    ctx,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast_deobfuscate::DeobfuscateState;
    use oxc_allocator::Allocator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_semantic::SemanticBuilder;
    use oxc_span::SourceType;
    use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

    fn run_unicode_normalizer(code: &str) -> String {
        let allocator = Allocator::default();
        let source_type = SourceType::mjs();
        let ret = Parser::new(&allocator, code, source_type).parse();
        let mut program = ret.program;

        let mut normalizer = UnicodeNormalizer::new();
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

        traverse_mut_with_ctx(&mut normalizer, &mut program, &mut ctx);

        Codegen::new().build(&program).code
    }

    #[test]
    fn test_normalize_zero_width_in_string() {
        let output = run_unicode_normalizer("var x = \"hello\u{200B}world\";");
        eprintln!("Output: {output}");
        assert!(
            !output.contains('\u{200B}'),
            "Should remove zero-width chars, got: {output}"
        );
        assert!(
            output.contains("helloworld"),
            "String should be normalized, got: {output}"
        );
    }

    #[test]
    fn test_normalize_cyrillic_in_string() {
        let output = run_unicode_normalizer("var x = \"\u{0410}\u{0412}\u{0421}\";");
        eprintln!("Output: {output}");
        assert!(
            output.contains("ABC"),
            "Should normalize Cyrillic to ASCII, got: {output}"
        );
    }

    #[test]
    fn test_preserve_normal_strings() {
        let output = run_unicode_normalizer("var x = \"normal string\";");
        eprintln!("Output: {output}");
        assert!(
            output.contains("normal string"),
            "Should preserve normal strings, got: {output}"
        );
    }
}
