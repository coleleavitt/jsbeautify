use oxc_ast::ast::{BindingPattern, Expression, Statement, VariableDeclarator};

pub(super) fn decl_name<'a>(decl: &'a VariableDeclarator<'_>) -> Option<&'a str> {
    match &decl.id {
        BindingPattern::BindingIdentifier(b) => Some(b.name.as_str()),
        _ => None,
    }
}

/// Match `(function (exports, require, module, __filename, __dirname) { ... })(...)`.
/// Match `(function (exports, require, module, __filename, __dirname) { ... })(...)`.
/// OR the Bun form: `(function(exports, require, module, __filename, __dirname) { ... })`
/// without the trailing call (the IIFE isn't invoked, just defined as an expression-statement).
/// Returns the body statements if matched.
pub(super) fn unwrap_cjs_iife<'a, 'b>(expr: &'a Expression<'b>) -> Option<&'a [Statement<'b>]> {
    // Extract the FunctionExpression from various wrapping shapes.
    let func = if let Expression::CallExpression(call) = expr {
        // Case 1: Called IIFE: `(function(...){})(...)`
        match &call.callee {
            Expression::ParenthesizedExpression(p) => match &p.expression {
                Expression::FunctionExpression(f) => Some(f.as_ref()),
                _ => None,
            },
            Expression::FunctionExpression(f) => Some(f.as_ref()),
            _ => None,
        }
    } else if let Expression::ParenthesizedExpression(p) = expr {
        // Case 2: Non-called wrapper: `(function(...){})`
        match &p.expression {
            Expression::FunctionExpression(f) => Some(f.as_ref()),
            _ => None,
        }
    } else if let Expression::FunctionExpression(f) = expr {
        Some(f.as_ref())
    } else {
        None
    }?;

    let body = func.body.as_ref()?;
    // Must look like a CJS wrapper: at least 3 of {exports, require, module, __filename, __dirname}.
    let cjs_hits = func
        .params
        .items
        .iter()
        .filter_map(|p| match &p.pattern {
            BindingPattern::BindingIdentifier(b) => Some(b.name.as_str()),
            _ => None,
        })
        .filter(|n| matches!(*n, "exports" | "require" | "module" | "__filename" | "__dirname"))
        .count();
    if cjs_hits < 3 {
        return None;
    }
    Some(&body.statements)
}
