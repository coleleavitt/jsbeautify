use oxc_ast::ast::{Expression, Statement};

pub(in crate::ast_deobfuscate::cff_unflattener) fn body_references_param(
    stmts: &[Statement<'_>],
    param_name: &str,
) -> bool {
    stmts.iter().any(|s| stmt_references(s, param_name))
}

fn stmt_references(stmt: &Statement<'_>, name: &str) -> bool {
    match stmt {
        Statement::ExpressionStatement(es) => expr_references(&es.expression, name),
        Statement::ReturnStatement(r) => r.argument.as_ref().is_some_and(|e| expr_references(e, name)),
        Statement::VariableDeclaration(d) => d
            .declarations
            .iter()
            .any(|decl| decl.init.as_ref().is_some_and(|e| expr_references(e, name))),
        Statement::BlockStatement(b) => b.body.iter().any(|s| stmt_references(s, name)),
        Statement::IfStatement(i) => {
            expr_references(&i.test, name)
                || stmt_references(&i.consequent, name)
                || i.alternate.as_ref().is_some_and(|a| stmt_references(a, name))
        }
        Statement::ForStatement(f) => {
            f.test.as_ref().is_some_and(|e| expr_references(e, name))
                || f.update.as_ref().is_some_and(|e| expr_references(e, name))
                || stmt_references(&f.body, name)
        }
        Statement::ForInStatement(f) => expr_references(&f.right, name) || stmt_references(&f.body, name),
        Statement::ForOfStatement(f) => expr_references(&f.right, name) || stmt_references(&f.body, name),
        Statement::WhileStatement(w) => expr_references(&w.test, name) || stmt_references(&w.body, name),
        Statement::DoWhileStatement(d) => stmt_references(&d.body, name) || expr_references(&d.test, name),
        Statement::SwitchStatement(s) => {
            expr_references(&s.discriminant, name)
                || s.cases.iter().any(|c| {
                    c.test.as_ref().is_some_and(|e| expr_references(e, name))
                        || c.consequent.iter().any(|st| stmt_references(st, name))
                })
        }
        Statement::TryStatement(t) => {
            t.block.body.iter().any(|s| stmt_references(s, name))
                || t.handler
                    .as_ref()
                    .is_some_and(|h| h.body.body.iter().any(|s| stmt_references(s, name)))
                || t.finalizer
                    .as_ref()
                    .is_some_and(|f| f.body.iter().any(|s| stmt_references(s, name)))
        }
        Statement::ThrowStatement(t) => expr_references(&t.argument, name),
        Statement::LabeledStatement(l) => stmt_references(&l.body, name),
        _ => false,
    }
}

fn expr_references(expr: &Expression<'_>, name: &str) -> bool {
    match expr {
        Expression::Identifier(id) => id.name.as_str() == name,
        Expression::CallExpression(c) => {
            expr_references(&c.callee, name)
                || c.arguments
                    .iter()
                    .any(|a| a.as_expression().is_some_and(|e| expr_references(e, name)))
        }
        Expression::NewExpression(n) => {
            expr_references(&n.callee, name)
                || n.arguments
                    .iter()
                    .any(|a| a.as_expression().is_some_and(|e| expr_references(e, name)))
        }
        Expression::StaticMemberExpression(s) => expr_references(&s.object, name),
        Expression::ComputedMemberExpression(c) => {
            expr_references(&c.object, name) || expr_references(&c.expression, name)
        }
        Expression::AssignmentExpression(a) => expr_references(&a.right, name),
        Expression::BinaryExpression(b) => expr_references(&b.left, name) || expr_references(&b.right, name),
        Expression::LogicalExpression(l) => expr_references(&l.left, name) || expr_references(&l.right, name),
        Expression::UnaryExpression(u) => expr_references(&u.argument, name),
        Expression::UpdateExpression(u) => matches!(
            &u.argument,
            oxc_ast::ast::SimpleAssignmentTarget::AssignmentTargetIdentifier(id) if id.name.as_str() == name
        ),
        Expression::ConditionalExpression(c) => {
            expr_references(&c.test, name)
                || expr_references(&c.consequent, name)
                || expr_references(&c.alternate, name)
        }
        Expression::SequenceExpression(s) => s.expressions.iter().any(|e| expr_references(e, name)),
        Expression::ParenthesizedExpression(p) => expr_references(&p.expression, name),
        Expression::ArrayExpression(a) => a
            .elements
            .iter()
            .any(|el| el.as_expression().is_some_and(|e| expr_references(e, name))),
        Expression::ObjectExpression(o) => o.properties.iter().any(|prop| {
            if let oxc_ast::ast::ObjectPropertyKind::ObjectProperty(p) = prop {
                expr_references(&p.value, name)
            } else {
                false
            }
        }),
        Expression::FunctionExpression(f) => f
            .body
            .as_ref()
            .is_some_and(|b| b.statements.iter().any(|s| stmt_references(s, name))),
        Expression::ArrowFunctionExpression(a) => a.body.statements.iter().any(|s| stmt_references(s, name)),
        Expression::TemplateLiteral(t) => t.expressions.iter().any(|e| expr_references(e, name)),
        Expression::TaggedTemplateExpression(t) => {
            expr_references(&t.tag, name) || t.quasi.expressions.iter().any(|e| expr_references(e, name))
        }
        Expression::YieldExpression(y) => y.argument.as_ref().is_some_and(|e| expr_references(e, name)),
        Expression::AwaitExpression(a) => expr_references(&a.argument, name),
        _ => false,
    }
}
