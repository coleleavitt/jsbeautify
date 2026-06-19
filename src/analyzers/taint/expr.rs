use std::collections::HashSet;

use oxc_ast::ast::Expression;

/// True if `expr` reads a known taint source.
pub(super) fn is_taint_source(expr: &Expression<'_>) -> bool {
    match expr {
        // location.hash / location.search / location.href / document.URL / document.cookie
        Expression::StaticMemberExpression(m) => {
            let prop = m.property.name.as_str();
            let obj = base_object_name(&m.object);
            matches!(
                (obj.as_deref(), prop),
                (Some("location"), "hash" | "search" | "href" | "pathname")
                    | (
                        Some("document"),
                        "URL" | "documentURI" | "cookie" | "referrer" | "baseURI"
                    )
                    | (Some("window"), "name")
            ) || prop == "data" && obj.as_deref() == Some("event") // message event .data
        }
        // new URLSearchParams(location.search).get(...) and the like read sources too,
        // but we keep it simple: a call whose callee/args contain a source.
        Expression::CallExpression(c) => {
            c.arguments
                .iter()
                .any(|a| a.as_expression().is_some_and(is_taint_source))
                || expr_contains_source(&c.callee)
        }
        _ => false,
    }
}

/// The base identifier name of a (possibly nested) member expression object.
pub(super) fn base_object_name(expr: &Expression<'_>) -> Option<String> {
    match expr {
        Expression::Identifier(id) => Some(id.name.to_string()),
        Expression::StaticMemberExpression(m) => base_object_name(&m.object),
        Expression::ThisExpression(_) => Some("this".to_string()),
        _ => None,
    }
}

/// Recursively check whether `expr` reads a taint source anywhere.
pub(super) fn expr_contains_source(expr: &Expression<'_>) -> bool {
    if is_taint_source(expr) {
        return true;
    }
    match expr {
        Expression::StaticMemberExpression(m) => expr_contains_source(&m.object),
        Expression::BinaryExpression(b) => expr_contains_source(&b.left) || expr_contains_source(&b.right),
        Expression::TemplateLiteral(t) => t.expressions.iter().any(expr_contains_source),
        Expression::CallExpression(c) => {
            expr_contains_source(&c.callee)
                || c.arguments
                    .iter()
                    .any(|a| a.as_expression().is_some_and(expr_contains_source))
        }
        Expression::ParenthesizedExpression(p) => expr_contains_source(&p.expression),
        _ => false,
    }
}

/// Whether `expr` references any tainted binding name (or a direct source).
pub(super) fn expr_is_tainted(expr: &Expression<'_>, tainted: &HashSet<String>) -> bool {
    if expr_contains_source(expr) {
        return true;
    }
    match expr {
        Expression::Identifier(id) => tainted.contains(id.name.as_str()),
        Expression::StaticMemberExpression(m) => expr_is_tainted(&m.object, tainted),
        Expression::BinaryExpression(b) => expr_is_tainted(&b.left, tainted) || expr_is_tainted(&b.right, tainted),
        Expression::TemplateLiteral(t) => t.expressions.iter().any(|e| expr_is_tainted(e, tainted)),
        Expression::ParenthesizedExpression(p) => expr_is_tainted(&p.expression, tainted),
        Expression::CallExpression(c) => c
            .arguments
            .iter()
            .any(|a| a.as_expression().is_some_and(|e| expr_is_tainted(e, tainted))),
        Expression::LogicalExpression(l) => expr_is_tainted(&l.left, tainted) || expr_is_tainted(&l.right, tainted),
        Expression::ConditionalExpression(c) => {
            expr_is_tainted(&c.consequent, tainted) || expr_is_tainted(&c.alternate, tainted)
        }
        _ => false,
    }
}
