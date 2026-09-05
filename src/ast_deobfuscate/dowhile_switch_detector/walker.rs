use oxc_allocator::Vec as OxcVec;
use oxc_ast::ast::{BindingPattern, Expression, Statement, VariableDeclarator};

use super::extraction::try_extract_dowhile_dispatcher;
use super::types::{DoWhileDispatcherInfo, DoWhileDispatcherMap};

pub(super) fn walk_stmts<'a>(stmts: &OxcVec<'a, Statement<'a>>, map: &mut DoWhileDispatcherMap) {
    for stmt in stmts {
        walk_stmt(stmt, map);
    }
}

fn walk_stmt(stmt: &Statement<'_>, map: &mut DoWhileDispatcherMap) {
    match stmt {
        Statement::FunctionDeclaration(func) => {
            if let Some(info) = try_extract_dowhile_dispatcher(func, None) {
                insert_dispatcher(map, info);
            }
            if let Some(body) = &func.body {
                walk_stmts(&body.statements, map);
            }
        }
        Statement::VariableDeclaration(decl) => {
            for d in &decl.declarations {
                check_var_declarator(d, map);
            }
        }
        Statement::ExpressionStatement(es) => {
            walk_expr(&es.expression, map);
        }
        Statement::BlockStatement(b) => walk_stmts(&b.body, map),
        Statement::IfStatement(ifs) => {
            walk_stmt(&ifs.consequent, map);
            if let Some(alt) = &ifs.alternate {
                walk_stmt(alt, map);
            }
        }
        Statement::TryStatement(t) => {
            walk_stmts(&t.block.body, map);
            if let Some(h) = &t.handler {
                walk_stmts(&h.body.body, map);
            }
            if let Some(f) = &t.finalizer {
                walk_stmts(&f.body, map);
            }
        }
        Statement::ForStatement(f) => walk_stmt(&f.body, map),
        Statement::WhileStatement(w) => walk_stmt(&w.body, map),
        Statement::DoWhileStatement(d) => walk_stmt(&d.body, map),
        Statement::SwitchStatement(s) => {
            for case in &s.cases {
                walk_stmts(&case.consequent, map);
            }
        }
        Statement::ForInStatement(f) => walk_stmt(&f.body, map),
        Statement::ForOfStatement(f) => walk_stmt(&f.body, map),
        Statement::LabeledStatement(l) => walk_stmt(&l.body, map),
        _ => {}
    }
}

fn walk_expr(expr: &Expression<'_>, map: &mut DoWhileDispatcherMap) {
    match expr {
        Expression::FunctionExpression(func) => {
            if let Some(body) = &func.body {
                walk_stmts(&body.statements, map);
            }
        }
        Expression::ArrowFunctionExpression(func) => {
            if let Some(body) = func.body.as_function_body() {
                walk_stmts(&body.statements, map);
            } else if let Some(expr) = func.body.as_expression() {
                walk_expr(expr, map);
            }
        }
        Expression::AssignmentExpression(assign) => {
            walk_expr(&assign.right, map);
        }
        Expression::CallExpression(call) => {
            walk_expr(&call.callee, map);
            for arg in &call.arguments {
                if let Some(e) = arg.as_expression() {
                    walk_expr(e, map);
                }
            }
        }
        Expression::NewExpression(ne) => {
            walk_expr(&ne.callee, map);
            for arg in &ne.arguments {
                if let Some(e) = arg.as_expression() {
                    walk_expr(e, map);
                }
            }
        }
        Expression::SequenceExpression(seq) => {
            for e in &seq.expressions {
                walk_expr(e, map);
            }
        }
        Expression::ParenthesizedExpression(p) => walk_expr(&p.expression, map),
        Expression::ConditionalExpression(c) => {
            walk_expr(&c.consequent, map);
            walk_expr(&c.alternate, map);
        }
        Expression::LogicalExpression(l) => {
            walk_expr(&l.left, map);
            walk_expr(&l.right, map);
        }
        Expression::ObjectExpression(o) => {
            for prop in &o.properties {
                if let oxc_ast::ast::ObjectPropertyKind::ObjectProperty(p) = prop {
                    walk_expr(&p.value, map);
                }
            }
        }
        Expression::ArrayExpression(a) => {
            for elem in &a.elements {
                if let Some(e) = elem.as_expression() {
                    walk_expr(e, map);
                }
            }
        }
        Expression::UnaryExpression(u) => walk_expr(&u.argument, map),
        Expression::BinaryExpression(b) => {
            walk_expr(&b.left, map);
            walk_expr(&b.right, map);
        }
        Expression::StaticMemberExpression(s) => walk_expr(&s.object, map),
        Expression::ComputedMemberExpression(c) => {
            walk_expr(&c.object, map);
            walk_expr(&c.expression, map);
        }
        Expression::TaggedTemplateExpression(t) => walk_expr(&t.tag, map),
        _ => {}
    }
}

fn check_var_declarator(declarator: &VariableDeclarator<'_>, map: &mut DoWhileDispatcherMap) {
    let BindingPattern::BindingIdentifier(var_id) = &declarator.id else {
        return;
    };
    let Some(Expression::FunctionExpression(func)) = &declarator.init else {
        if let Some(init) = &declarator.init {
            walk_expr(init, map);
        }
        return;
    };
    let var_name = var_id.name.as_str().to_string();
    if let Some(info) = try_extract_dowhile_dispatcher(func, Some(var_name)) {
        insert_dispatcher(map, info);
    }
    if let Some(body) = &func.body {
        walk_stmts(&body.statements, map);
    }
}

fn insert_dispatcher(map: &mut DoWhileDispatcherMap, info: DoWhileDispatcherInfo) {
    if let Some(alt) = &info.alt_name {
        map.insert(
            alt.clone(),
            DoWhileDispatcherInfo {
                name: alt.clone(),
                alt_name: Some(info.name.clone()),
                state_param: info.state_param.clone(),
                args_param: info.args_param.clone(),
                exit_sentinel: info.exit_sentinel.clone(),
                cases: Vec::new(),
                has_default: info.has_default,
            },
        );
    }
    map.insert(info.name.clone(), info);
}
