use oxc_allocator::{CloneIn, Vec as OxcVec};
use oxc_ast::ast::{BindingPattern, Expression, Function, Program, Statement, VariableDeclarator};
use rustc_hash::FxHashMap;

use super::CaseBodyMap;
use crate::ast_deobfuscate::dispatcher_detector::DispatcherMap;

mod refs;

pub(super) use refs::body_references_param;

pub fn collect_case_bodies<'a>(
    program: &Program<'a>,
    dispatchers: &DispatcherMap,
    alloc: &'a oxc_allocator::Allocator,
) -> CaseBodyMap<'a> {
    let mut bodies = FxHashMap::default();
    scan_stmts(&program.body, dispatchers, &mut bodies, alloc);
    bodies
}

fn scan_stmts<'a>(
    stmts: &OxcVec<'a, Statement<'a>>,
    dispatchers: &DispatcherMap,
    bodies: &mut CaseBodyMap<'a>,
    alloc: &'a oxc_allocator::Allocator,
) {
    for stmt in stmts {
        scan_stmt(stmt, dispatchers, bodies, alloc);
    }
}

fn scan_stmt<'a>(
    stmt: &Statement<'a>,
    dispatchers: &DispatcherMap,
    bodies: &mut CaseBodyMap<'a>,
    alloc: &'a oxc_allocator::Allocator,
) {
    match stmt {
        Statement::FunctionDeclaration(func) => {
            if let Some(id) = &func.id {
                let name = id.name.as_str();
                if let Some(info) = dispatchers.get(name) {
                    extract_case_bodies(func, &info.name, dispatchers, bodies, alloc);
                }
            }
            if let Some(body) = &func.body {
                scan_stmts(&body.statements, dispatchers, bodies, alloc);
            }
        }
        Statement::VariableDeclaration(decl) => {
            for d in &decl.declarations {
                scan_var_declarator(d, dispatchers, bodies, alloc);
            }
        }
        Statement::ExpressionStatement(es) => {
            scan_expr(&es.expression, dispatchers, bodies, alloc);
        }
        Statement::BlockStatement(b) => scan_stmts(&b.body, dispatchers, bodies, alloc),
        Statement::IfStatement(ifs) => {
            scan_stmt(&ifs.consequent, dispatchers, bodies, alloc);
            if let Some(alt) = &ifs.alternate {
                scan_stmt(alt, dispatchers, bodies, alloc);
            }
        }
        Statement::TryStatement(t) => {
            scan_stmts(&t.block.body, dispatchers, bodies, alloc);
            if let Some(h) = &t.handler {
                scan_stmts(&h.body.body, dispatchers, bodies, alloc);
            }
            if let Some(f) = &t.finalizer {
                scan_stmts(&f.body, dispatchers, bodies, alloc);
            }
        }
        Statement::ForStatement(f) => scan_stmt(&f.body, dispatchers, bodies, alloc),
        Statement::ForInStatement(f) => scan_stmt(&f.body, dispatchers, bodies, alloc),
        Statement::ForOfStatement(f) => scan_stmt(&f.body, dispatchers, bodies, alloc),
        Statement::WhileStatement(w) => scan_stmt(&w.body, dispatchers, bodies, alloc),
        Statement::DoWhileStatement(d) => scan_stmt(&d.body, dispatchers, bodies, alloc),
        Statement::LabeledStatement(l) => scan_stmt(&l.body, dispatchers, bodies, alloc),
        Statement::SwitchStatement(s) => {
            for case in &s.cases {
                scan_stmts(&case.consequent, dispatchers, bodies, alloc);
            }
        }
        _ => {}
    }
}

fn scan_expr<'a>(
    expr: &Expression<'a>,
    dispatchers: &DispatcherMap,
    bodies: &mut CaseBodyMap<'a>,
    alloc: &'a oxc_allocator::Allocator,
) {
    match expr {
        Expression::FunctionExpression(func) => {
            if let Some(body) = &func.body {
                scan_stmts(&body.statements, dispatchers, bodies, alloc);
            }
        }
        Expression::ArrowFunctionExpression(func) => {
            scan_stmts(&func.body.statements, dispatchers, bodies, alloc);
        }
        Expression::AssignmentExpression(assign) => {
            scan_expr(&assign.right, dispatchers, bodies, alloc);
        }
        Expression::CallExpression(call) => {
            scan_expr(&call.callee, dispatchers, bodies, alloc);
            for arg in &call.arguments {
                if let Some(e) = arg.as_expression() {
                    scan_expr(e, dispatchers, bodies, alloc);
                }
            }
        }
        Expression::NewExpression(ne) => {
            scan_expr(&ne.callee, dispatchers, bodies, alloc);
            for arg in &ne.arguments {
                if let Some(e) = arg.as_expression() {
                    scan_expr(e, dispatchers, bodies, alloc);
                }
            }
        }
        Expression::SequenceExpression(seq) => {
            for e in &seq.expressions {
                scan_expr(e, dispatchers, bodies, alloc);
            }
        }
        Expression::ParenthesizedExpression(p) => {
            scan_expr(&p.expression, dispatchers, bodies, alloc);
        }
        Expression::ConditionalExpression(c) => {
            scan_expr(&c.consequent, dispatchers, bodies, alloc);
            scan_expr(&c.alternate, dispatchers, bodies, alloc);
        }
        Expression::LogicalExpression(l) => {
            scan_expr(&l.left, dispatchers, bodies, alloc);
            scan_expr(&l.right, dispatchers, bodies, alloc);
        }
        Expression::ObjectExpression(o) => {
            for prop in &o.properties {
                if let oxc_ast::ast::ObjectPropertyKind::ObjectProperty(p) = prop {
                    scan_expr(&p.value, dispatchers, bodies, alloc);
                }
            }
        }
        Expression::ArrayExpression(a) => {
            for elem in &a.elements {
                if let Some(e) = elem.as_expression() {
                    scan_expr(e, dispatchers, bodies, alloc);
                }
            }
        }
        Expression::UnaryExpression(u) => {
            scan_expr(&u.argument, dispatchers, bodies, alloc);
        }
        Expression::BinaryExpression(b) => {
            scan_expr(&b.left, dispatchers, bodies, alloc);
            scan_expr(&b.right, dispatchers, bodies, alloc);
        }
        Expression::StaticMemberExpression(s) => {
            scan_expr(&s.object, dispatchers, bodies, alloc);
        }
        Expression::ComputedMemberExpression(c) => {
            scan_expr(&c.object, dispatchers, bodies, alloc);
            scan_expr(&c.expression, dispatchers, bodies, alloc);
        }
        Expression::TaggedTemplateExpression(t) => {
            scan_expr(&t.tag, dispatchers, bodies, alloc);
        }
        _ => {}
    }
}

fn scan_var_declarator<'a>(
    declarator: &VariableDeclarator<'a>,
    dispatchers: &DispatcherMap,
    bodies: &mut CaseBodyMap<'a>,
    alloc: &'a oxc_allocator::Allocator,
) {
    let BindingPattern::BindingIdentifier(var_id) = &declarator.id else {
        return;
    };
    let Some(Expression::FunctionExpression(func)) = &declarator.init else {
        return;
    };
    let var_name = var_id.name.as_str();
    if let Some(info) = dispatchers.get(var_name) {
        extract_case_bodies(func, &info.name, dispatchers, bodies, alloc);
    }
    // Always recurse into the function body to find nested dispatchers
    if let Some(body) = &func.body {
        scan_stmts(&body.statements, dispatchers, bodies, alloc);
    }
}

fn extract_case_bodies<'a>(
    func: &Function<'a>,
    dispatcher_name: &str,
    dispatchers: &DispatcherMap,
    bodies: &mut CaseBodyMap<'a>,
    alloc: &'a oxc_allocator::Allocator,
) {
    let Some(body) = &func.body else { return };
    let switch = body.statements.iter().find_map(|s| {
        if let Statement::SwitchStatement(sw) = s {
            Some(sw)
        } else {
            None
        }
    });
    let Some(switch) = switch else { return };
    let Some(info) = dispatchers.get(dispatcher_name) else {
        return;
    };

    for case in &switch.cases {
        let Some(Expression::Identifier(label_id)) = &case.test else {
            continue;
        };
        let label = label_id.name.as_str();
        if !info.cases.contains_key(label) {
            continue;
        }
        let mut stmts: OxcVec<'a, Statement<'a>> = OxcVec::new_in(alloc);
        for s in &case.consequent {
            if matches!(s, Statement::BreakStatement(_)) {
                continue;
            }
            stmts.push(s.clone_in_with_semantic_ids(alloc));
        }
        bodies.insert((dispatcher_name.to_string(), label.to_string()), stmts);
    }
}
