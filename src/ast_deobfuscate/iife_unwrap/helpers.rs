use oxc_allocator::CloneIn;
use oxc_ast::ast::{ArrowFunctionExpression, Expression, Statement, VariableDeclaration, VariableDeclarator};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use std::cell::Cell;

use super::Ctx;

fn extract_zero_arg_arrow_iife<'a, 'b>(expr: &'b Expression<'a>) -> Option<&'b ArrowFunctionExpression<'a>> {
    let Expression::CallExpression(call) = expr else {
        return None;
    };
    if !call.arguments.is_empty() {
        return None;
    }
    let Expression::ParenthesizedExpression(paren) = &call.callee else {
        return None;
    };
    let Expression::ArrowFunctionExpression(arrow) = &paren.expression else {
        return None;
    };
    if !arrow.params.items.is_empty() {
        return None;
    }
    if arrow.expression {
        return None;
    }
    if arrow.r#async {
        return None;
    }
    Some(arrow)
}

pub(super) fn has_iife_candidate(stmt: &Statement<'_>) -> bool {
    if let Statement::ExpressionStatement(expr_stmt) = stmt
        && extract_zero_arg_arrow_iife(&expr_stmt.expression).is_some()
    {
        return true;
    }
    if let Statement::VariableDeclaration(var_decl) = stmt
        && var_decl.declarations.len() == 1
        && let Some(init) = &var_decl.declarations[0].init
        && extract_zero_arg_arrow_iife(init).is_some()
    {
        return true;
    }
    false
}

pub(super) fn try_unwrap_standalone<'a>(
    stmt: &Statement<'a>,
    new_body: &mut oxc_allocator::Vec<'a, Statement<'a>>,
    ctx: &mut Ctx<'a>,
) -> bool {
    let Statement::ExpressionStatement(expr_stmt) = stmt else {
        return false;
    };
    let Some(arrow) = extract_zero_arg_arrow_iife(&expr_stmt.expression) else {
        return false;
    };
    if arrow.body.statements.iter().any(|s| contains_return(s)) {
        return false;
    }
    for inner_stmt in &arrow.body.statements {
        new_body.push(inner_stmt.clone_in_with_semantic_ids(ctx.ast.allocator));
    }
    true
}

fn contains_return(stmt: &Statement<'_>) -> bool {
    match stmt {
        Statement::ReturnStatement(_) => true,
        Statement::IfStatement(if_stmt) => {
            if contains_return(&if_stmt.consequent) {
                return true;
            }
            if let Some(alt) = &if_stmt.alternate
                && contains_return(alt)
            {
                return true;
            }
            false
        }
        Statement::BlockStatement(block) => block.body.iter().any(|s| contains_return(s)),
        Statement::ForStatement(f) => contains_return(&f.body),
        Statement::ForInStatement(f) => contains_return(&f.body),
        Statement::ForOfStatement(f) => contains_return(&f.body),
        Statement::WhileStatement(w) => contains_return(&w.body),
        Statement::DoWhileStatement(d) => contains_return(&d.body),
        Statement::SwitchStatement(sw) => sw.cases.iter().any(|c| c.consequent.iter().any(|s| contains_return(s))),
        Statement::TryStatement(t) => {
            if t.block.body.iter().any(|s| contains_return(s)) {
                return true;
            }
            if let Some(handler) = &t.handler
                && handler.body.body.iter().any(|s| contains_return(s))
            {
                return true;
            }
            if let Some(finalizer) = &t.finalizer
                && finalizer.body.iter().any(|s| contains_return(s))
            {
                return true;
            }
            false
        }
        Statement::LabeledStatement(l) => contains_return(&l.body),
        Statement::WithStatement(w) => contains_return(&w.body),
        _ => false,
    }
}

pub(super) fn try_unwrap_assigned<'a>(
    stmt: &Statement<'a>,
    new_body: &mut oxc_allocator::Vec<'a, Statement<'a>>,
    ctx: &mut Ctx<'a>,
) -> bool {
    let Statement::VariableDeclaration(var_decl) = stmt else {
        return false;
    };
    if var_decl.declarations.len() != 1 {
        return false;
    }
    let Some(init) = &var_decl.declarations[0].init else {
        return false;
    };
    let Some(arrow) = extract_zero_arg_arrow_iife(init) else {
        return false;
    };
    let body_stmts = &arrow.body.statements;
    if body_stmts.is_empty() {
        return false;
    }
    let last_idx = body_stmts.len().wrapping_sub(1);
    let Statement::ReturnStatement(ret) = &body_stmts[last_idx] else {
        return false;
    };
    let Some(return_expr) = &ret.argument else {
        return false;
    };
    let has_early_return = body_stmts[..last_idx].iter().any(|s| contains_return(s));
    if has_early_return {
        return false;
    }
    for inner_stmt in &body_stmts[..last_idx] {
        new_body.push(inner_stmt.clone_in_with_semantic_ids(ctx.ast.allocator));
    }
    let new_init = return_expr.clone_in_with_semantic_ids(ctx.ast.allocator);
    let new_declarator = VariableDeclarator {
        node_id: Cell::new(NodeId::DUMMY),
        span: SPAN,
        kind: var_decl.kind,
        id: var_decl.declarations[0]
            .id
            .clone_in_with_semantic_ids(ctx.ast.allocator),
        type_annotation: None,
        init: Some(new_init),
        definite: false,
    };
    let mut new_declarations = ctx.ast.vec();
    new_declarations.push(new_declarator);
    let new_var = Statement::VariableDeclaration(ctx.ast.alloc(VariableDeclaration {
        node_id: Cell::new(NodeId::DUMMY),
        span: SPAN,
        kind: var_decl.kind,
        declarations: new_declarations,
        declare: var_decl.declare,
    }));
    new_body.push(new_var);
    true
}
