use oxc_allocator::GetAllocator;
use oxc_ast::ast::{Expression, TemplateElement, TemplateElementValue};
use oxc_span::SPAN;

use super::Ctx;

pub(super) fn make_string_literal<'a>(value: &str, ctx: &Ctx<'a>) -> Expression<'a> {
    Expression::new_string_literal(SPAN, ctx.allocator().alloc_str(value), None, ctx)
}

fn escape_template_raw(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '`' => escaped.push_str("\\`"),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            _ => escaped.push(ch),
        }
    }
    escaped.replace("${", "\\${")
}

pub(super) fn make_template_element<'a>(value: &str, tail: bool, ctx: &Ctx<'a>) -> TemplateElement<'a> {
    let cooked = oxc_ast::ast::Str::from_str_in(ctx.allocator().alloc_str(value), ctx);
    let raw_text = escape_template_raw(value);
    let raw = oxc_ast::ast::Str::from_str_in(&raw_text, ctx);
    TemplateElement::new(SPAN, TemplateElementValue { cooked: Some(cooked), raw }, tail, ctx)
}
