use oxc_allocator::Vec as OxcVec;
use oxc_ast::ast::{BindingPattern, Expression, Statement};

use super::{AkamaiDetector, BMP_IDENT_SIGS, is_self_init_accessor};

// Pure-scan walker used only by the detector (no mutation, no Oxc traversal).
pub(in crate::ast_deobfuscate::akamai) fn walk_detector<'a>(
    stmts: &OxcVec<'a, Statement<'a>>,
    detector: &mut AkamaiDetector,
) {
    for stmt in stmts {
        walk_detector_stmt(stmt, detector);
    }
}

fn walk_detector_stmt(stmt: &Statement<'_>, detector: &mut AkamaiDetector) {
    match stmt {
        Statement::VariableDeclaration(decl) => {
            for d in &decl.declarations {
                if let BindingPattern::BindingIdentifier(id) = &d.id {
                    let name = id.name.as_str();
                    if BMP_IDENT_SIGS.contains(&name) {
                        detector.record(name);
                    }
                }
                if let Some(init) = &d.init {
                    walk_detector_expr(init, detector);
                }
            }
        }
        Statement::ExpressionStatement(es) => walk_detector_expr(&es.expression, detector),
        Statement::BlockStatement(b) => walk_detector(&b.body, detector),
        Statement::FunctionDeclaration(func) => {
            if is_self_init_accessor(func)
                && let Some(name) = func.id.as_ref().map(|id| id.name.as_str())
            {
                detector.record_accessor(name);
            }
            if let Some(body) = &func.body {
                walk_detector(&body.statements, detector);
            }
        }
        Statement::IfStatement(ifs) => {
            walk_detector_expr(&ifs.test, detector);
            walk_detector_stmt(&ifs.consequent, detector);
            if let Some(alt) = &ifs.alternate {
                walk_detector_stmt(alt, detector);
            }
        }
        Statement::ReturnStatement(ret) => {
            if let Some(arg) = &ret.argument {
                walk_detector_expr(arg, detector);
            }
        }
        Statement::TryStatement(t) => {
            walk_detector(&t.block.body, detector);
            if let Some(h) = &t.handler {
                walk_detector(&h.body.body, detector);
            }
            if let Some(f) = &t.finalizer {
                walk_detector(&f.body, detector);
            }
        }
        Statement::ForStatement(f) => walk_detector_stmt(&f.body, detector),
        Statement::WhileStatement(w) => walk_detector_stmt(&w.body, detector),
        Statement::DoWhileStatement(d) => walk_detector_stmt(&d.body, detector),
        Statement::SwitchStatement(s) => {
            for case in &s.cases {
                walk_detector(&case.consequent, detector);
            }
        }
        _ => {}
    }
}

fn walk_detector_expr(expr: &Expression<'_>, detector: &mut AkamaiDetector) {
    match expr {
        Expression::Identifier(id) => {
            let name = id.name.as_str();
            if BMP_IDENT_SIGS.contains(&name) {
                detector.record(name);
            }
        }
        Expression::StringLiteral(s) => {
            let v = s.value.as_str();
            if BMP_IDENT_SIGS.contains(&v) {
                detector.record(v);
            }
        }
        Expression::CallExpression(call) => {
            walk_detector_expr(&call.callee, detector);
            for arg in &call.arguments {
                if let Some(e) = arg.as_expression() {
                    walk_detector_expr(e, detector);
                }
            }
        }
        Expression::FunctionExpression(func) => {
            if is_self_init_accessor(func)
                && let Some(name) = func.id.as_ref().map(|id| id.name.as_str())
            {
                detector.record_accessor(name);
            }
            if let Some(body) = &func.body {
                walk_detector(&body.statements, detector);
            }
        }
        Expression::ArrowFunctionExpression(func) => {
            if let Some(body) = func.body.as_function_body() {
                walk_detector(&body.statements, detector);
            } else if let Some(expr) = func.body.as_expression() {
                walk_detector_expr(expr, detector);
            }
        }
        Expression::StaticMemberExpression(sme) => walk_detector_expr(&sme.object, detector),
        Expression::ComputedMemberExpression(cme) => {
            walk_detector_expr(&cme.object, detector);
            walk_detector_expr(&cme.expression, detector);
        }
        Expression::BinaryExpression(b) => {
            walk_detector_expr(&b.left, detector);
            walk_detector_expr(&b.right, detector);
        }
        Expression::LogicalExpression(l) => {
            walk_detector_expr(&l.left, detector);
            walk_detector_expr(&l.right, detector);
        }
        Expression::UnaryExpression(u) => walk_detector_expr(&u.argument, detector),
        Expression::ConditionalExpression(c) => {
            walk_detector_expr(&c.test, detector);
            walk_detector_expr(&c.consequent, detector);
            walk_detector_expr(&c.alternate, detector);
        }
        Expression::SequenceExpression(seq) => {
            for e in &seq.expressions {
                walk_detector_expr(e, detector);
            }
        }
        Expression::AssignmentExpression(a) => walk_detector_expr(&a.right, detector),
        Expression::ArrayExpression(arr) => {
            for el in &arr.elements {
                if let Some(e) = el.as_expression() {
                    walk_detector_expr(e, detector);
                }
            }
        }
        Expression::ParenthesizedExpression(paren) => walk_detector_expr(&paren.expression, detector),
        _ => {}
    }
}
