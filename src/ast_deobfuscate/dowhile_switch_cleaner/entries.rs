use oxc_allocator::Vec as OxcVec;
use oxc_ast::ast::{Expression, Program, Statement};
use rustc_hash::{FxHashMap, FxHashSet};

use super::DoWhileDispatcherMap;

pub(super) fn collect_entry_states(
    program: &Program<'_>,
    dispatchers: &DoWhileDispatcherMap,
) -> FxHashMap<String, FxHashSet<String>> {
    let mut entries: FxHashMap<String, FxHashSet<String>> = FxHashMap::default();
    scan_stmts_for_entries(&program.body, dispatchers, &mut entries);
    entries
}

fn scan_stmts_for_entries<'a>(
    stmts: &OxcVec<'a, Statement<'a>>,
    dispatchers: &DoWhileDispatcherMap,
    entries: &mut FxHashMap<String, FxHashSet<String>>,
) {
    for stmt in stmts {
        scan_stmt_for_entries(stmt, dispatchers, entries);
    }
}

fn scan_stmt_for_entries(
    stmt: &Statement<'_>,
    dispatchers: &DoWhileDispatcherMap,
    entries: &mut FxHashMap<String, FxHashSet<String>>,
) {
    match stmt {
        Statement::ExpressionStatement(es) => scan_expr_for_entries(&es.expression, dispatchers, entries),
        Statement::VariableDeclaration(decl) => {
            for d in &decl.declarations {
                if let Some(init) = &d.init {
                    scan_expr_for_entries(init, dispatchers, entries);
                }
            }
        }
        Statement::ReturnStatement(r) => {
            if let Some(arg) = &r.argument {
                scan_expr_for_entries(arg, dispatchers, entries);
            }
        }
        Statement::FunctionDeclaration(func) => {
            if let Some(body) = &func.body {
                scan_stmts_for_entries(&body.statements, dispatchers, entries);
            }
        }
        Statement::BlockStatement(b) => scan_stmts_for_entries(&b.body, dispatchers, entries),
        Statement::IfStatement(ifs) => {
            scan_expr_for_entries(&ifs.test, dispatchers, entries);
            scan_stmt_for_entries(&ifs.consequent, dispatchers, entries);
            if let Some(alt) = &ifs.alternate {
                scan_stmt_for_entries(alt, dispatchers, entries);
            }
        }
        Statement::ForStatement(f) => scan_stmt_for_entries(&f.body, dispatchers, entries),
        Statement::ForInStatement(f) => scan_stmt_for_entries(&f.body, dispatchers, entries),
        Statement::ForOfStatement(f) => scan_stmt_for_entries(&f.body, dispatchers, entries),
        Statement::WhileStatement(w) => scan_stmt_for_entries(&w.body, dispatchers, entries),
        Statement::DoWhileStatement(d) => scan_stmt_for_entries(&d.body, dispatchers, entries),
        Statement::SwitchStatement(s) => {
            for case in &s.cases {
                scan_stmts_for_entries(&case.consequent, dispatchers, entries);
            }
        }
        Statement::TryStatement(t) => {
            scan_stmts_for_entries(&t.block.body, dispatchers, entries);
            if let Some(h) = &t.handler {
                scan_stmts_for_entries(&h.body.body, dispatchers, entries);
            }
            if let Some(f) = &t.finalizer {
                scan_stmts_for_entries(&f.body, dispatchers, entries);
            }
        }
        Statement::LabeledStatement(l) => scan_stmt_for_entries(&l.body, dispatchers, entries),
        _ => {}
    }
}

fn scan_expr_for_entries(
    expr: &Expression<'_>,
    dispatchers: &DoWhileDispatcherMap,
    entries: &mut FxHashMap<String, FxHashSet<String>>,
) {
    match expr {
        Expression::CallExpression(call) => {
            if let Some((disp_name, state_label)) = extract_dispatcher_call(call, dispatchers) {
                entries.entry(disp_name).or_default().insert(state_label);
            }
            scan_expr_for_entries(&call.callee, dispatchers, entries);
            for arg in &call.arguments {
                if let Some(e) = arg.as_expression() {
                    scan_expr_for_entries(e, dispatchers, entries);
                }
            }
        }
        Expression::FunctionExpression(func) => {
            if let Some(body) = &func.body {
                scan_stmts_for_entries(&body.statements, dispatchers, entries);
            }
        }
        Expression::ArrowFunctionExpression(func) => {
            scan_stmts_for_entries(&func.body.statements, dispatchers, entries);
        }
        Expression::AssignmentExpression(a) => scan_expr_for_entries(&a.right, dispatchers, entries),
        Expression::BinaryExpression(b) => {
            scan_expr_for_entries(&b.left, dispatchers, entries);
            scan_expr_for_entries(&b.right, dispatchers, entries);
        }
        Expression::LogicalExpression(l) => {
            scan_expr_for_entries(&l.left, dispatchers, entries);
            scan_expr_for_entries(&l.right, dispatchers, entries);
        }
        Expression::ConditionalExpression(c) => {
            scan_expr_for_entries(&c.test, dispatchers, entries);
            scan_expr_for_entries(&c.consequent, dispatchers, entries);
            scan_expr_for_entries(&c.alternate, dispatchers, entries);
        }
        Expression::SequenceExpression(s) => {
            for e in &s.expressions {
                scan_expr_for_entries(e, dispatchers, entries);
            }
        }
        Expression::ParenthesizedExpression(p) => {
            scan_expr_for_entries(&p.expression, dispatchers, entries);
        }
        Expression::UnaryExpression(u) => scan_expr_for_entries(&u.argument, dispatchers, entries),
        Expression::ArrayExpression(a) => {
            for elem in &a.elements {
                if let Some(e) = elem.as_expression() {
                    scan_expr_for_entries(e, dispatchers, entries);
                }
            }
        }
        Expression::ObjectExpression(o) => {
            for prop in &o.properties {
                if let oxc_ast::ast::ObjectPropertyKind::ObjectProperty(p) = prop {
                    scan_expr_for_entries(&p.value, dispatchers, entries);
                }
            }
        }
        Expression::StaticMemberExpression(s) => scan_expr_for_entries(&s.object, dispatchers, entries),
        Expression::ComputedMemberExpression(c) => {
            scan_expr_for_entries(&c.object, dispatchers, entries);
            scan_expr_for_entries(&c.expression, dispatchers, entries);
        }
        Expression::NewExpression(n) => {
            scan_expr_for_entries(&n.callee, dispatchers, entries);
            for arg in &n.arguments {
                if let Some(e) = arg.as_expression() {
                    scan_expr_for_entries(e, dispatchers, entries);
                }
            }
        }
        _ => {}
    }
}

fn extract_dispatcher_call(
    call: &oxc_ast::ast::CallExpression<'_>,
    dispatchers: &DoWhileDispatcherMap,
) -> Option<(String, String)> {
    // Direct: DISPATCHER(STATE, args)
    if let Expression::Identifier(callee_id) = &call.callee {
        let name = callee_id.name.as_str();
        if dispatchers.contains_key(name)
            && call.arguments.len() == 2
            && let Some(Expression::Identifier(state_id)) = call.arguments[0].as_expression()
        {
            return Some((name.to_string(), state_id.name.as_str().to_string()));
        }
    }
    // .call(this, STATE, args) form
    if let Expression::StaticMemberExpression(sme) = &call.callee
        && sme.property.name.as_str() == "call"
        && let Expression::Identifier(obj_id) = &sme.object
    {
        let name = obj_id.name.as_str();
        if dispatchers.contains_key(name)
            && call.arguments.len() == 3
            && let Some(Expression::Identifier(state_id)) = call.arguments[1].as_expression()
        {
            return Some((name.to_string(), state_id.name.as_str().to_string()));
        }
    }
    None
}
