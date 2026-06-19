use oxc_allocator::CloneIn;
use oxc_ast::ast::{Expression, StringLiteral, TemplateElement, TemplateElementValue};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use std::cell::Cell;

use super::Ctx;

pub(super) fn make_string_literal<'a>(value: &str, ctx: &Ctx<'a>) -> Expression<'a> {
    let interned = ctx.ast.str(value).clone_in(ctx.ast.allocator);
    Expression::StringLiteral(ctx.ast.alloc(StringLiteral {
        node_id: Cell::new(NodeId::DUMMY),
        span: SPAN,
        value: interned,
        raw: None,
        lone_surrogates: false,
    }))
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
    let cooked = ctx.ast.str(value);
    let raw_text = escape_template_raw(value);
    let raw = ctx.ast.str(&raw_text);
    TemplateElement {
        node_id: Cell::new(NodeId::DUMMY),
        span: SPAN,
        tail,
        value: TemplateElementValue {
            cooked: Some(cooked),
            raw,
        },
        lone_surrogates: false,
    }
}
