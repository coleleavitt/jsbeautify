use oxc_allocator::Vec as OxcVec;
use oxc_ast::ast::{
    AssignmentOperator, AssignmentTarget, BinaryOperator, BindingPattern, Expression, Program, Statement, UnaryOperator,
};
use rustc_hash::FxHashMap;
use std::hash::BuildHasher;

use super::deferred_const::DeferredConst;
use super::types::{CompoundOp, DoWhileDispatcherInfo, StateTransition};

fn try_eval_expr(expr: &Expression<'_>, known: &FxHashMap<String, i64>) -> Option<i64> {
    match expr {
        Expression::NumericLiteral(n) => {
            let v = n.value;
            if v.fract() != 0.0 || v > i64::MAX as f64 || v < i64::MIN as f64 {
                return None;
            }
            Some(v as i64)
        }
        Expression::Identifier(id) => known.get(id.name.as_str()).copied(),
        Expression::BinaryExpression(bin) => {
            if bin.operator == BinaryOperator::Addition
                && let Some(concat) = try_eval_array_concat(&bin.left, &bin.right, known)
            {
                return Some(concat);
            }
            let l = try_eval_expr(&bin.left, known)?;
            let r = try_eval_expr(&bin.right, known)?;
            match bin.operator {
                BinaryOperator::Addition => l.checked_add(r),
                BinaryOperator::Subtraction => l.checked_sub(r),
                BinaryOperator::Multiplication => l.checked_mul(r),
                _ => None,
            }
        }
        Expression::UnaryExpression(u) if u.operator == UnaryOperator::UnaryPlus => try_eval_expr(&u.argument, known),
        Expression::ParenthesizedExpression(p) => try_eval_expr(&p.expression, known),
        Expression::ArrayExpression(arr) => {
            if arr.elements.is_empty() {
                return Some(0);
            }
            if arr.elements.len() == 1
                && let Some(e) = arr.elements[0].as_expression()
            {
                return try_eval_expr(e, known);
            }
            None
        }
        _ => None,
    }
}

/// Handles JS `[N] + [M]` → string concat → numeric: `"NM"` parsed as i64.
fn try_eval_array_concat(left: &Expression<'_>, right: &Expression<'_>, known: &FxHashMap<String, i64>) -> Option<i64> {
    let l_str = try_eval_array_as_string(left, known)?;
    let r_str = try_eval_array_as_string(right, known)?;
    let concat = format!("{l_str}{r_str}");
    concat.parse::<i64>().ok()
}

fn try_eval_array_as_string(expr: &Expression<'_>, known: &FxHashMap<String, i64>) -> Option<String> {
    let Expression::ArrayExpression(arr) = expr else {
        return None;
    };
    if arr.elements.is_empty() {
        return Some(String::new());
    }
    if arr.elements.len() == 1
        && let Some(e) = arr.elements[0].as_expression()
    {
        let v = try_eval_expr(e, known)?;
        return Some(v.to_string());
    }
    None
}

/// Collect numeric constants from `var X = EXPR;` and `X = EXPR;` assignments.
pub fn collect_constants(program: &Program<'_>) -> FxHashMap<String, i64> {
    let mut known: FxHashMap<String, i64> = FxHashMap::default();
    let mut deferred: Vec<(String, DeferredConst)> = Vec::new();

    collect_constants_from_stmts(&program.body, &mut known, &mut deferred);

    for _ in 0..10 {
        let prev = known.len();
        deferred.retain(|(name, expr)| {
            if let Some(v) = expr.eval(&known) {
                known.insert(name.clone(), v);
                return false;
            }
            true
        });
        if known.len() == prev {
            break;
        }
    }

    known
}

fn collect_constants_from_stmts<'a>(
    stmts: &OxcVec<'a, Statement<'a>>,
    known: &mut FxHashMap<String, i64>,
    deferred: &mut Vec<(String, DeferredConst)>,
) {
    for stmt in stmts {
        collect_constants_from_stmt(stmt, known, deferred);
    }
}

fn collect_constants_from_stmt(
    stmt: &Statement<'_>,
    known: &mut FxHashMap<String, i64>,
    deferred: &mut Vec<(String, DeferredConst)>,
) {
    match stmt {
        Statement::VariableDeclaration(decl) => {
            for d in &decl.declarations {
                let BindingPattern::BindingIdentifier(id) = &d.id else {
                    if let Some(init) = &d.init {
                        collect_constants_from_expr(init, known, deferred);
                    }
                    continue;
                };
                let Some(init) = &d.init else { continue };
                let name = id.name.as_str().to_string();
                if let Some(v) = try_eval_expr(init, known) {
                    known.insert(name, v);
                } else if let Some(dc) = DeferredConst::from_ast(init) {
                    deferred.push((name, dc));
                } else {
                    collect_constants_from_expr(init, known, deferred);
                }
            }
        }
        Statement::ExpressionStatement(es) => {
            if let Expression::AssignmentExpression(assign) = &es.expression {
                if let AssignmentTarget::AssignmentTargetIdentifier(target) = &assign.left {
                    let name = target.name.as_str().to_string();
                    if assign.operator == AssignmentOperator::Assign {
                        if let Some(v) = try_eval_expr(&assign.right, known) {
                            known.insert(name, v);
                        } else if let Some(dc) = DeferredConst::from_ast(&assign.right) {
                            deferred.push((name, dc));
                        } else {
                            collect_constants_from_expr(&assign.right, known, deferred);
                        }
                    }
                } else {
                    collect_constants_from_expr(&es.expression, known, deferred);
                }
            } else {
                collect_constants_from_expr(&es.expression, known, deferred);
            }
        }
        Statement::FunctionDeclaration(func) => {
            if let Some(body) = &func.body {
                collect_constants_from_stmts(&body.statements, known, deferred);
            }
        }
        Statement::BlockStatement(b) => collect_constants_from_stmts(&b.body, known, deferred),
        Statement::IfStatement(ifs) => {
            collect_constants_from_stmt(&ifs.consequent, known, deferred);
            if let Some(alt) = &ifs.alternate {
                collect_constants_from_stmt(alt, known, deferred);
            }
        }
        Statement::TryStatement(t) => {
            collect_constants_from_stmts(&t.block.body, known, deferred);
            if let Some(h) = &t.handler {
                collect_constants_from_stmts(&h.body.body, known, deferred);
            }
            if let Some(f) = &t.finalizer {
                collect_constants_from_stmts(&f.body, known, deferred);
            }
        }
        Statement::ForStatement(f) => collect_constants_from_stmt(&f.body, known, deferred),
        Statement::WhileStatement(w) => collect_constants_from_stmt(&w.body, known, deferred),
        Statement::DoWhileStatement(d) => collect_constants_from_stmt(&d.body, known, deferred),
        Statement::SwitchStatement(s) => {
            for case in &s.cases {
                collect_constants_from_stmts(&case.consequent, known, deferred);
            }
        }
        Statement::ForInStatement(f) => collect_constants_from_stmt(&f.body, known, deferred),
        Statement::ForOfStatement(f) => collect_constants_from_stmt(&f.body, known, deferred),
        Statement::LabeledStatement(l) => collect_constants_from_stmt(&l.body, known, deferred),
        _ => {}
    }
}

fn collect_constants_from_expr(
    expr: &Expression<'_>,
    known: &mut FxHashMap<String, i64>,
    deferred: &mut Vec<(String, DeferredConst)>,
) {
    match expr {
        Expression::FunctionExpression(func) => {
            if let Some(body) = &func.body {
                collect_constants_from_stmts(&body.statements, known, deferred);
            }
        }
        Expression::ArrowFunctionExpression(func) => {
            collect_constants_from_stmts(&func.body.statements, known, deferred);
        }
        Expression::AssignmentExpression(assign) => {
            collect_constants_from_expr(&assign.right, known, deferred);
        }
        Expression::CallExpression(call) => {
            collect_constants_from_expr(&call.callee, known, deferred);
        }
        Expression::SequenceExpression(seq) => {
            for e in &seq.expressions {
                collect_constants_from_expr(e, known, deferred);
            }
        }
        Expression::ParenthesizedExpression(p) => {
            collect_constants_from_expr(&p.expression, known, deferred);
        }
        _ => {}
    }
}

/// Resolve `Compound` transitions to `Sequential` using a constant dictionary.
pub fn resolve_compound_transitions<S: BuildHasher>(
    info: &mut DoWhileDispatcherInfo,
    constants: &std::collections::HashMap<String, i64, S>,
) -> usize {
    let mut label_values: FxHashMap<String, i64> = FxHashMap::default();
    for case in &info.cases {
        if let Some(&val) = constants.get(&case.label) {
            label_values.insert(case.label.clone(), val);
        }
    }

    let mut value_to_label: FxHashMap<i64, String> = FxHashMap::default();
    for (label, &val) in &label_values {
        value_to_label.insert(val, label.clone());
    }

    let mut resolved = 0;
    for case in &mut info.cases {
        if let StateTransition::Compound { op, ref rhs_name } = case.transition {
            let Some(&src_val) = label_values.get(&case.label) else {
                continue;
            };
            let Some(&rhs_val) = constants.get(rhs_name.as_str()) else {
                continue;
            };
            let target_val = match op {
                CompoundOp::AddAssign => src_val.wrapping_add(rhs_val),
                CompoundOp::SubAssign => src_val.wrapping_sub(rhs_val),
            };
            if let Some(target_label) = value_to_label.get(&target_val) {
                case.transition = StateTransition::Sequential(target_label.clone());
                resolved += 1;
            }
        }
    }
    resolved
}
