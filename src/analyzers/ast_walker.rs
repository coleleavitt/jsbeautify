//! Common oxc helpers for analyzers.
//!
//! Each analyzer is a self-contained function `fn analyze(source) -> Vec<Match>`
//! that owns its own parse + walk. This avoids trait-object lifetime hell and
//! lets analyzers be cheaply composed at the call site.

use oxc_allocator::Allocator;
use oxc_ast::ast::Program;
use oxc_parser::Parser;
use oxc_span::SourceType;

use super::match_kind::PositionRange;

/// Parse `source` once and hand the program to a visitor closure.
///
/// The closure receives the parsed `Program` and the original source string
/// (so it can compute line/column positions from byte offsets).
pub fn parse_with<F, R>(source: &str, f: F) -> R
where
    F: for<'a> FnOnce(&'a Allocator, &'a Program<'a>, &str) -> R,
    R: Default,
{
    let allocator = Allocator::default();
    let parse_result = Parser::new(&allocator, source, SourceType::mjs()).parse();

    if parse_result.program.body.is_empty() && !parse_result.errors.is_empty() {
        // Couldn't parse anything useful.
        return R::default();
    }

    // SAFETY: We need to extend the lifetime of program's borrow to allocator's.
    // We do this by re-parsing inside the closure body via &allocator.
    // Simpler: inline parse here.
    f(&allocator, &parse_result.program, source)
}

/// Convert oxc byte offsets to a [`PositionRange`].
#[must_use]
pub fn span_to_pos(source: &str, start: u32, end: u32) -> PositionRange {
    PositionRange::from_byte_range(source, start as usize, end as usize)
}

/// Helper: extract just the static-text segments of a template literal.
///
/// `${expr}` segments are dropped — only the literal parts of
/// `` `path /api/${user}/items` `` come back as `["path /api/", "/items"]`.
#[must_use]
pub fn template_static_parts<'a>(lit: &oxc_ast::ast::TemplateLiteral<'a>) -> Vec<&'a str> {
    lit.quasis
        .iter()
        .filter_map(|q| q.value.cooked.as_ref().map(oxc_ast::ast::Str::as_str))
        .collect()
}

/// Try to read the static-string value out of an [`Expression`], if it is
/// a literal string or a fully-static template literal.
#[must_use]
pub fn expression_as_static_string(expr: &oxc_ast::ast::Expression<'_>) -> Option<String> {
    use oxc_ast::ast::Expression;
    match expr {
        Expression::StringLiteral(s) => Some(s.value.to_string()),
        Expression::TemplateLiteral(t) if t.expressions.is_empty() => Some(
            t.quasis
                .iter()
                .filter_map(|q| q.value.cooked.as_ref().map(oxc_ast::ast::Str::as_str))
                .collect::<Vec<_>>()
                .join(""),
        ),
        _ => None,
    }
}

/// Get the `name` of a static identifier or string property key.
#[must_use]
pub fn property_key_name<'a>(key: &oxc_ast::ast::PropertyKey<'a>) -> Option<&'a str> {
    use oxc_ast::ast::PropertyKey;
    match key {
        PropertyKey::StaticIdentifier(id) => Some(id.name.as_str()),
        PropertyKey::StringLiteral(s) => Some(s.value.as_str()),
        PropertyKey::PrivateIdentifier(p) => Some(p.name.as_str()),
        _ => None,
    }
}
