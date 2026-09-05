use oxc_allocator::Vec as OxcVec;
use oxc_ast::ast::{ArrayExpressionElement, BindingPattern, Expression, Statement};
use rustc_hash::FxHashMap;

#[derive(Default, Debug)]
pub(super) struct TrackerStats {
    pub(super) declared: bool,
    /// `Ot.push(...)` / `Ot.pop(...)` / `Ot.splice(...)` calls.
    pub(super) write_calls: usize,
    /// Any other use — read access, comparison, assignment other than these calls.
    pub(super) reads: usize,
}

impl TrackerStats {
    pub(super) const fn is_pure_tracker(&self) -> bool {
        self.declared && self.reads == 0 && self.write_calls >= 4
    }
}

pub(super) fn collect_tracker_stats<'a>(stmts: &OxcVec<'a, Statement<'a>>, map: &mut FxHashMap<String, TrackerStats>) {
    for stmt in stmts {
        scan_stmt_for_tracker(stmt, map);
    }
}

fn scan_stmt_for_tracker(stmt: &Statement<'_>, map: &mut FxHashMap<String, TrackerStats>) {
    match stmt {
        Statement::VariableDeclaration(decl) => {
            for d in &decl.declarations {
                if let BindingPattern::BindingIdentifier(id) = &d.id
                    && d.init.is_none()
                {
                    // `var Ot;` — candidate.
                    map.entry(id.name.as_str().to_string()).or_default().declared = true;
                }
                if let Some(init) = &d.init {
                    scan_expr_for_tracker(init, map);
                }
            }
        }
        Statement::ExpressionStatement(es) => scan_expr_for_tracker(&es.expression, map),
        Statement::ReturnStatement(ret) => {
            if let Some(arg) = &ret.argument {
                scan_expr_for_tracker(arg, map);
            }
        }
        Statement::IfStatement(ifs) => {
            scan_expr_for_tracker(&ifs.test, map);
            scan_stmt_for_tracker(&ifs.consequent, map);
            if let Some(alt) = &ifs.alternate {
                scan_stmt_for_tracker(alt, map);
            }
        }
        Statement::BlockStatement(block) => collect_tracker_stats(&block.body, map),
        Statement::TryStatement(t) => {
            collect_tracker_stats(&t.block.body, map);
            if let Some(h) = &t.handler {
                collect_tracker_stats(&h.body.body, map);
            }
            if let Some(f) = &t.finalizer {
                collect_tracker_stats(&f.body, map);
            }
        }
        Statement::ForStatement(f) => {
            if let Some(body) = &f.init {
                if let oxc_ast::ast::ForStatementInit::VariableDeclaration(vd) = body {
                    for d in &vd.declarations {
                        if let Some(init) = &d.init {
                            scan_expr_for_tracker(init, map);
                        }
                    }
                } else if let Some(expr) = body.as_expression() {
                    scan_expr_for_tracker(expr, map);
                }
            }
            scan_stmt_for_tracker(&f.body, map);
        }
        Statement::WhileStatement(w) => {
            scan_expr_for_tracker(&w.test, map);
            scan_stmt_for_tracker(&w.body, map);
        }
        Statement::DoWhileStatement(d) => {
            scan_expr_for_tracker(&d.test, map);
            scan_stmt_for_tracker(&d.body, map);
        }
        Statement::SwitchStatement(s) => {
            scan_expr_for_tracker(&s.discriminant, map);
            for case in &s.cases {
                if let Some(t) = &case.test {
                    scan_expr_for_tracker(t, map);
                }
                collect_tracker_stats(&case.consequent, map);
            }
        }
        Statement::FunctionDeclaration(func) => {
            if let Some(body) = &func.body {
                collect_tracker_stats(&body.statements, map);
            }
        }
        _ => {}
    }
}

fn scan_expr_for_tracker(expr: &Expression<'_>, map: &mut FxHashMap<String, TrackerStats>) {
    match expr {
        Expression::CallExpression(call) => {
            // Check for `<ident>.{push,pop,splice}(...)` tracker write.
            if let Expression::StaticMemberExpression(sme) = &call.callee
                && let Expression::Identifier(id) = &sme.object
                && matches!(sme.property.name.as_str(), "push" | "pop" | "splice")
            {
                map.entry(id.name.as_str().to_string()).or_default().write_calls += 1;
                // Arguments may contain other expressions; scan them.
                for arg in &call.arguments {
                    if let Some(e) = arg.as_expression() {
                        scan_expr_for_tracker(e, map);
                    }
                }
                return;
            }
            // Any call: scan callee & args normally.
            scan_expr_for_tracker(&call.callee, map);
            for arg in &call.arguments {
                if let Some(e) = arg.as_expression() {
                    scan_expr_for_tracker(e, map);
                }
            }
        }
        Expression::Identifier(id) => {
            map.entry(id.name.as_str().to_string()).or_default().reads += 1;
        }
        Expression::ComputedMemberExpression(cme) => {
            scan_expr_for_tracker(&cme.object, map);
            scan_expr_for_tracker(&cme.expression, map);
        }
        Expression::StaticMemberExpression(sme) => {
            // NB: `sme.object` counts as a read.
            scan_expr_for_tracker(&sme.object, map);
        }
        Expression::BinaryExpression(b) => {
            scan_expr_for_tracker(&b.left, map);
            scan_expr_for_tracker(&b.right, map);
        }
        Expression::LogicalExpression(l) => {
            scan_expr_for_tracker(&l.left, map);
            scan_expr_for_tracker(&l.right, map);
        }
        Expression::UnaryExpression(u) => scan_expr_for_tracker(&u.argument, map),
        Expression::ConditionalExpression(c) => {
            scan_expr_for_tracker(&c.test, map);
            scan_expr_for_tracker(&c.consequent, map);
            scan_expr_for_tracker(&c.alternate, map);
        }
        Expression::SequenceExpression(seq) => {
            for e in &seq.expressions {
                scan_expr_for_tracker(e, map);
            }
        }
        Expression::AssignmentExpression(a) => {
            scan_expr_for_tracker(&a.right, map);
        }
        Expression::ArrayExpression(arr) => {
            for el in &arr.elements {
                if let ArrayExpressionElement::SpreadElement(s) = el {
                    scan_expr_for_tracker(&s.argument, map);
                } else if let Some(e) = el.as_expression() {
                    scan_expr_for_tracker(e, map);
                }
            }
        }
        Expression::ObjectExpression(obj) => {
            for prop in &obj.properties {
                if let oxc_ast::ast::ObjectPropertyKind::ObjectProperty(p) = prop {
                    scan_expr_for_tracker(&p.value, map);
                }
            }
        }
        Expression::FunctionExpression(func) => {
            if let Some(body) = &func.body {
                collect_tracker_stats(&body.statements, map);
            }
        }
        Expression::ArrowFunctionExpression(func) => {
            if let Some(body) = func.body.as_function_body() {
                collect_tracker_stats(&body.statements, map);
            } else if let Some(expr) = func.body.as_expression() {
                scan_expr_for_tracker(expr, map);
            }
        }
        Expression::ParenthesizedExpression(paren) => scan_expr_for_tracker(&paren.expression, map),
        _ => {}
    }
}
