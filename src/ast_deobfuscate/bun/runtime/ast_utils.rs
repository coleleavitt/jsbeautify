use oxc_ast::ast::{
    ArrowFunctionExpression, BindingIdentifier, BindingPattern, Expression, FunctionBody, Statement, VariableDeclarator,
};
use oxc_ast_visit::{Visit, walk};

pub(super) fn decl_binding<'a, 'b>(decl: &'a VariableDeclarator<'b>) -> Option<&'a BindingIdentifier<'b>> {
    match &decl.id {
        BindingPattern::BindingIdentifier(b) => Some(b),
        _ => None,
    }
}

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

/// The body of a parameterless IIFE, `(() => { … })()` or `(function () { … })()`,
/// the wrapper of an esbuild `--format=iife` bundle.
pub(super) fn unwrap_iife<'a, 'b>(expr: &'a Expression<'b>) -> Option<&'a [Statement<'b>]> {
    let Expression::CallExpression(call) = expr.without_parentheses() else {
        return None;
    };
    if !call.arguments.is_empty() {
        return None;
    }
    let body = match call.callee.without_parentheses() {
        Expression::ArrowFunctionExpression(arrow) if arrow.params.is_empty() => arrow.body.as_function_body()?,
        Expression::FunctionExpression(func) if func.params.is_empty() => func.body.as_ref()?,
        _ => return None,
    };
    Some(&body.statements)
}

/// The parameter names of `arrow` when every parameter is a plain identifier.
pub(super) fn param_names<'a>(arrow: &'a ArrowFunctionExpression<'_>) -> Option<Vec<&'a str>> {
    if arrow.params.rest.is_some() {
        return None;
    }
    arrow
        .params
        .items
        .iter()
        .map(|param| param.pattern.get_binding_identifier().map(|id| id.name.as_str()))
        .collect()
}

/// The statements of a block body, or `None` for an expression body.
pub(super) fn block_body<'a, 'b>(arrow: &'a ArrowFunctionExpression<'b>) -> Option<&'a FunctionBody<'b>> {
    arrow.body.as_function_body()
}

/// The parenthesis-free expression of an expression body.
pub(super) fn expression_body<'a, 'b>(arrow: &'a ArrowFunctionExpression<'b>) -> Option<&'a Expression<'b>> {
    arrow.body.as_expression().map(Expression::without_parentheses)
}

/// Whether some expression inside `arrow` (its body included) satisfies `pred`.
pub(super) fn arrow_contains<'a>(
    arrow: &ArrowFunctionExpression<'a>,
    pred: impl FnMut(&Expression<'a>) -> bool,
) -> bool {
    let mut finder = ExpressionFinder { pred, found: false };
    finder.visit_arrow_function_expression(arrow);
    finder.found
}

/// Whether `expr` or some expression inside it satisfies `pred`.
pub(super) fn expression_contains<'a>(expr: &Expression<'a>, pred: impl FnMut(&Expression<'a>) -> bool) -> bool {
    let mut finder = ExpressionFinder { pred, found: false };
    finder.visit_expression(expr);
    finder.found
}

/// Whether `arrow` contains the string literal `value`.
pub(super) fn arrow_contains_string(arrow: &ArrowFunctionExpression<'_>, value: &str) -> bool {
    arrow_contains(arrow, |expr| is_string(expr, value))
}

pub(super) fn is_string(expr: &Expression<'_>, value: &str) -> bool {
    matches!(expr, Expression::StringLiteral(lit) if lit.value.as_str() == value)
}

pub(super) fn is_ident(expr: &Expression<'_>, name: &str) -> bool {
    matches!(expr.without_parentheses(), Expression::Identifier(id) if id.name.as_str() == name)
}

struct ExpressionFinder<F> {
    pred: F,
    found: bool,
}

impl<'a, F: FnMut(&Expression<'a>) -> bool> Visit<'a> for ExpressionFinder<F> {
    fn visit_expression(&mut self, expr: &Expression<'a>) {
        if self.found {
            return;
        }
        if (self.pred)(expr) {
            self.found = true;
            return;
        }
        walk::walk_expression(self, expr);
    }
}
