use oxc_ast::ast::{Expression, Statement};

pub(super) fn body_contains_esmodule_check(stmts: &[Statement<'_>]) -> bool {
    for stmt in stmts {
        if stmt_str_contains(stmt, "__esModule") {
            return true;
        }
    }
    false
}

pub(super) fn body_contains_esmodule_string(stmts: &[Statement<'_>]) -> bool {
    body_contains_esmodule_check(stmts)
}

pub(super) fn body_calls_define_property_with_name(stmts: &[Statement<'_>]) -> bool {
    // Look for ExpressionStatement -> CallExpression with one argument that's the string "name".
    for stmt in stmts {
        if let Statement::ExpressionStatement(es) = stmt
            && call_has_string_arg(&es.expression, "name")
        {
            return true;
        }
    }
    false
}

fn call_has_string_arg(expr: &Expression<'_>, target: &str) -> bool {
    let Expression::CallExpression(call) = expr else {
        return false;
    };
    call.arguments.iter().any(|a| {
        if let oxc_ast::ast::Argument::StringLiteral(s) = a {
            s.value.as_str() == target
        } else {
            false
        }
    })
}

/// Recursively check if a statement subtree contains a string literal matching `target`.
fn stmt_str_contains(stmt: &Statement<'_>, target: &str) -> bool {
    contains_str_in_stmt(stmt, target)
}

fn contains_str_in_stmt(stmt: &Statement<'_>, target: &str) -> bool {
    match stmt {
        Statement::ExpressionStatement(es) => contains_str_in_expr(&es.expression, target),
        Statement::ReturnStatement(rs) => rs.argument.as_ref().is_some_and(|e| contains_str_in_expr(e, target)),
        Statement::IfStatement(is) => {
            contains_str_in_expr(&is.test, target)
                || contains_str_in_stmt(&is.consequent, target)
                || is.alternate.as_ref().is_some_and(|s| contains_str_in_stmt(s, target))
        }
        Statement::BlockStatement(bs) => bs.body.iter().any(|s| contains_str_in_stmt(s, target)),
        Statement::VariableDeclaration(vd) => vd
            .declarations
            .iter()
            .any(|d| d.init.as_ref().is_some_and(|e| contains_str_in_expr(e, target))),
        Statement::ForStatement(fs) => contains_str_in_stmt(&fs.body, target),
        Statement::ForInStatement(fs) => contains_str_in_stmt(&fs.body, target),
        Statement::ForOfStatement(fs) => contains_str_in_stmt(&fs.body, target),
        _ => false,
    }
}

pub(super) fn stmt_returns_ident(stmt: &Statement<'_>, target: &str) -> bool {
    match stmt {
        Statement::ReturnStatement(ret) => {
            matches!(&ret.argument, Some(Expression::Identifier(id)) if id.name.as_str() == target)
        }
        Statement::BlockStatement(block) => block.body.iter().any(|stmt| stmt_returns_ident(stmt, target)),
        Statement::IfStatement(if_stmt) => {
            stmt_returns_ident(&if_stmt.consequent, target)
                || if_stmt
                    .alternate
                    .as_ref()
                    .is_some_and(|alternate| stmt_returns_ident(alternate, target))
        }
        _ => false,
    }
}

fn contains_str_in_expr(expr: &Expression<'_>, target: &str) -> bool {
    match expr {
        Expression::StringLiteral(s) => s.value.as_str() == target,
        Expression::ParenthesizedExpression(p) => contains_str_in_expr(&p.expression, target),
        Expression::SequenceExpression(s) => s.expressions.iter().any(|e| contains_str_in_expr(e, target)),
        Expression::CallExpression(c) => {
            contains_str_in_expr(&c.callee, target)
                || c.arguments.iter().any(|arg| {
                    arg.as_expression()
                        .is_some_and(|expr| contains_str_in_expr(expr, target))
                })
        }
        Expression::ObjectExpression(o) => o.properties.iter().any(|p| match p {
            oxc_ast::ast::ObjectPropertyKind::ObjectProperty(prop) => contains_str_in_expr(&prop.value, target),
            oxc_ast::ast::ObjectPropertyKind::SpreadProperty(_) => false,
        }),
        Expression::AssignmentExpression(a) => contains_str_in_expr(&a.right, target),
        Expression::BinaryExpression(b) => {
            contains_str_in_expr(&b.left, target) || contains_str_in_expr(&b.right, target)
        }
        Expression::LogicalExpression(l) => {
            contains_str_in_expr(&l.left, target) || contains_str_in_expr(&l.right, target)
        }
        Expression::ConditionalExpression(c) => {
            contains_str_in_expr(&c.test, target)
                || contains_str_in_expr(&c.consequent, target)
                || contains_str_in_expr(&c.alternate, target)
        }
        Expression::StaticMemberExpression(m) => {
            contains_str_in_expr(&m.object, target) || m.property.name.as_str() == target
        }
        Expression::UnaryExpression(u) => contains_str_in_expr(&u.argument, target),
        _ => false,
    }
}
