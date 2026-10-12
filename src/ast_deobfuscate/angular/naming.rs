use oxc_ast::ast::{ArrayExpressionElement, Expression, ObjectExpression, ObjectPropertyKind};

use super::DefKind;

pub(super) fn object_property<'b, 'a>(obj: &'b ObjectExpression<'a>, name: &str) -> Option<&'b Expression<'a>> {
    obj.properties.iter().find_map(|prop| match prop {
        ObjectPropertyKind::ObjectProperty(p) if p.key.static_name().is_some_and(|key| key == name) => Some(&p.value),
        _ => None,
    })
}

/// Class name for an Ivy definition object, e.g. `selectors: [["app-login"]]`
/// → `AppLoginComponent`, `name: "currency"` (pipe) → `CurrencyPipe`.
pub(super) fn class_name_for(kind: DefKind, def: &ObjectExpression<'_>) -> Option<String> {
    let base = match kind {
        DefKind::Pipe => match object_property(def, "name")? {
            Expression::StringLiteral(s) => s.value.to_string(),
            _ => return None,
        },
        DefKind::Component | DefKind::Directive => selector_base(object_property(def, "selectors")?)?,
    };
    let mut name = pascal_case(&base)?;
    if !name.ends_with(kind.suffix()) {
        name.push_str(kind.suffix());
    }
    Some(name)
}

/// The most specific simple name in the first compiled CSS selector:
/// `["app-login"]` (element), `["", "appHighlight", ""]` (attribute), or
/// `["", 8, "card"]` (class, `SelectorFlags.CLASS`).
fn selector_base(selectors: &Expression<'_>) -> Option<String> {
    let Expression::ArrayExpression(list) = selectors else {
        return None;
    };
    let Some(ArrayExpressionElement::ArrayExpression(first)) = list.elements.first() else {
        return None;
    };
    let parts: Vec<&Expression<'_>> = first.elements.iter().filter_map(|e| e.as_expression()).collect();
    let tag = match parts.first() {
        Some(Expression::StringLiteral(s)) => s.value.as_str(),
        _ => return None,
    };
    if !tag.is_empty() {
        return (tag != "ng-component").then(|| tag.to_string());
    }
    match parts.get(1..)? {
        [Expression::StringLiteral(attr), ..] => Some(attr.value.to_string()),
        [Expression::NumericLiteral(flag), Expression::StringLiteral(class), ..] if flag.value == 8.0 => {
            Some(class.value.to_string())
        }
        _ => None,
    }
}

/// `app-login` → `AppLogin`, `appHighlight` → `AppHighlight`. Returns `None`
/// when the result would not start with an ASCII letter.
fn pascal_case(raw: &str) -> Option<String> {
    let mut out = String::with_capacity(raw.len());
    for word in raw
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
    {
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            out.push(first.to_ascii_uppercase());
            out.push_str(chars.as_str());
        }
    }
    out.starts_with(|c: char| c.is_ascii_alphabetic()).then_some(out)
}

/// `NgRx` action type `"[Auth] Login Success"` → `loginSuccess`.
pub(super) fn action_name(action_type: &str) -> Option<String> {
    let rest = action_type.strip_prefix('[')?;
    let (source, event) = rest.split_once(']')?;
    if source.trim().is_empty() {
        return None;
    }
    let pascal = pascal_case(event.trim())?;
    let mut chars = pascal.chars();
    let first = chars.next()?;
    Some(first.to_ascii_lowercase().to_string() + chars.as_str())
}
