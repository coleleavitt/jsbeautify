use oxc_allocator::Vec as OxcVec;
use oxc_ast::ast::{BindingPattern, Expression, Statement, VariableDeclarator};
use rustc_hash::FxHashMap;

use super::extraction::try_extract_dispatcher;
use super::types::{DispatcherInfo, DispatcherMap};

pub(super) fn walk_stmts<'a>(stmts: &OxcVec<'a, Statement<'a>>, map: &mut DispatcherMap) {
    for stmt in stmts {
        walk_stmt(stmt, map);
    }
}

fn walk_stmt(stmt: &Statement<'_>, map: &mut DispatcherMap) {
    match stmt {
        Statement::FunctionDeclaration(func) => {
            let name = func.id.as_ref().map_or("<anon>", |id| id.name.as_str());
            cff_dbg!(
                "[DBG/disp] walk_stmt: FunctionDeclaration name={} params={}",
                name,
                func.params.items.len()
            );
            if let Some(info) = try_extract_dispatcher(func, None) {
                deob_trace!(
                    "[DISPATCHER] found {}({}, {}) with {} cases",
                    info.name,
                    info.state_param,
                    info.args_param,
                    info.cases.len()
                );
                map.insert(info.name.clone(), info);
            }
            if let Some(body) = &func.body {
                cff_dbg!(
                    "[DBG/disp] walk_stmt: recursing into FunctionDeclaration {} body ({} stmts)",
                    name,
                    body.statements.len()
                );
                walk_stmts(&body.statements, map);
            }
        }
        Statement::VariableDeclaration(decl) => {
            cff_dbg!(
                "[DBG/disp] walk_stmt: VariableDeclaration n={}",
                decl.declarations.len()
            );
            for d in &decl.declarations {
                check_var_declarator(d, map);
            }
        }
        Statement::ExpressionStatement(es) => {
            walk_expr(&es.expression, map);
        }
        Statement::BlockStatement(b) => {
            cff_dbg!("[DBG/disp] walk_stmt: BlockStatement n={}", b.body.len());
            walk_stmts(&b.body, map);
        }
        Statement::IfStatement(ifs) => {
            cff_dbg!("[DBG/disp] walk_stmt: IfStatement");
            walk_stmt(&ifs.consequent, map);
            if let Some(alt) = &ifs.alternate {
                walk_stmt(alt, map);
            }
        }
        Statement::TryStatement(t) => {
            cff_dbg!("[DBG/disp] walk_stmt: TryStatement");
            walk_stmts(&t.block.body, map);
            if let Some(h) = &t.handler {
                walk_stmts(&h.body.body, map);
            }
            if let Some(f) = &t.finalizer {
                walk_stmts(&f.body, map);
            }
        }
        Statement::ForStatement(f) => {
            cff_dbg!("[DBG/disp] walk_stmt: ForStatement");
            walk_stmt(&f.body, map);
        }
        Statement::WhileStatement(w) => {
            cff_dbg!("[DBG/disp] walk_stmt: WhileStatement");
            walk_stmt(&w.body, map);
        }
        Statement::DoWhileStatement(d) => {
            cff_dbg!("[DBG/disp] walk_stmt: DoWhileStatement, recursing into body");
            walk_stmt(&d.body, map);
        }
        Statement::SwitchStatement(s) => {
            cff_dbg!("[DBG/disp] walk_stmt: SwitchStatement with {} cases", s.cases.len());
            for case in &s.cases {
                walk_stmts(&case.consequent, map);
            }
        }
        Statement::ForInStatement(f) => {
            cff_dbg!("[DBG/disp] walk_stmt: ForInStatement");
            walk_stmt(&f.body, map);
        }
        Statement::ForOfStatement(f) => {
            cff_dbg!("[DBG/disp] walk_stmt: ForOfStatement");
            walk_stmt(&f.body, map);
        }
        Statement::LabeledStatement(l) => {
            cff_dbg!("[DBG/disp] walk_stmt: LabeledStatement label={}", l.label.name);
            walk_stmt(&l.body, map);
        }
        _ => {
            cff_dbg!(
                "[DBG/disp] walk_stmt: unhandled variant {:?}",
                std::mem::discriminant(stmt)
            );
        }
    }
}

/// Walk into expressions to find function expressions whose bodies may contain dispatchers.
fn walk_expr(expr: &Expression<'_>, map: &mut DispatcherMap) {
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
        Expression::ParenthesizedExpression(p) => {
            walk_expr(&p.expression, map);
        }
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
        Expression::UnaryExpression(u) => {
            walk_expr(&u.argument, map);
        }
        Expression::BinaryExpression(b) => {
            walk_expr(&b.left, map);
            walk_expr(&b.right, map);
        }
        Expression::StaticMemberExpression(s) => {
            walk_expr(&s.object, map);
        }
        Expression::ComputedMemberExpression(c) => {
            walk_expr(&c.object, map);
            walk_expr(&c.expression, map);
        }
        Expression::TaggedTemplateExpression(t) => {
            walk_expr(&t.tag, map);
        }
        _ => {}
    }
}

fn check_var_declarator(declarator: &VariableDeclarator<'_>, map: &mut DispatcherMap) {
    let BindingPattern::BindingIdentifier(var_id) = &declarator.id else {
        cff_dbg!("[DBG/disp] check_var_declarator: id is not BindingIdentifier, skipping");
        return;
    };
    let Some(Expression::FunctionExpression(func)) = &declarator.init else {
        cff_dbg!(
            "[DBG/disp] check_var_declarator: {} init is not FunctionExpression",
            var_id.name
        );
        return;
    };
    let var_name = var_id.name.as_str().to_string();
    cff_dbg!(
        "[DBG/disp] check_var_declarator: {} = function {}(...) with {} params",
        var_name,
        func.id.as_ref().map_or("<anon>", |id| id.name.as_str()),
        func.params.items.len()
    );
    if let Some(info) = try_extract_dispatcher(func, Some(var_name)) {
        deob_trace!(
            "[DISPATCHER] found {}({}, {}) with {} cases (alt_name={:?})",
            info.name,
            info.state_param,
            info.args_param,
            info.cases.len(),
            info.alt_name
        );
        if let Some(alt) = &info.alt_name {
            map.insert(
                alt.clone(),
                DispatcherInfo {
                    name: alt.clone(),
                    alt_name: Some(info.name.clone()),
                    state_param: info.state_param.clone(),
                    args_param: info.args_param.clone(),
                    cases: FxHashMap::default(),
                    has_default: info.has_default,
                },
            );
        }
        map.insert(info.name.clone(), info);
    }
    // Always recurse into the function body to find nested dispatchers
    if let Some(body) = &func.body {
        cff_dbg!(
            "[DBG/disp] check_var_declarator: recursing into {} body ({} stmts)",
            var_id.name,
            body.statements.len()
        );
        walk_stmts(&body.statements, map);
    }
}
