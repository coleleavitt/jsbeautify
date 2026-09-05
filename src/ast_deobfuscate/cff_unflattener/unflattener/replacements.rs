use oxc_allocator::{CloneIn, GetAllocator, Vec as OxcVec};
use oxc_ast::ast::{
    BindingPattern, Expression, FormalParameterKind, FormalParameters, FunctionBody, FunctionType, Statement,
    VariableDeclarationKind, VariableDeclarator,
};
use oxc_span::SPAN;

use super::super::case_bodies::body_references_param;
use crate::ast_deobfuscate::state::DeobfuscateState;

type Ctx<'a> = oxc_traverse::TraverseCtx<'a, DeobfuscateState>;

pub(super) fn make_iife<'a>(
    args_param_name: &str,
    body_stmts: &OxcVec<'a, Statement<'a>>,
    call_site_arg: &Expression<'a>,
    ctx: &Ctx<'a>,
) -> Expression<'a> {
    let mut stmts = OxcVec::new_in(ctx);
    if body_references_param(body_stmts.as_slice(), args_param_name) {
        stmts.push(make_args_declaration(args_param_name, call_site_arg, ctx));
    }
    stmts.extend(
        body_stmts
            .iter()
            .map(|stmt| stmt.clone_in_with_semantic_ids(ctx.allocator())),
    );

    let params = FormalParameters::boxed(SPAN, FormalParameterKind::FormalParameter, [], None, ctx);
    let body = FunctionBody::boxed(SPAN, [], stmts, ctx);
    let callee = Expression::new_function_expression(
        SPAN,
        FunctionType::FunctionExpression,
        None,
        false,
        false,
        false,
        None,
        None,
        params,
        None,
        Some(body),
        ctx,
    );
    Expression::new_call_expression(SPAN, callee, None, [], false, ctx)
}

pub(super) fn make_inline_stmts<'a>(
    args_param_name: &str,
    body_stmts: &OxcVec<'a, Statement<'a>>,
    call_site_arg: &Expression<'a>,
    ctx: &Ctx<'a>,
) -> OxcVec<'a, Statement<'a>> {
    let uses_args = body_references_param(body_stmts.as_slice(), args_param_name);
    let mut body = OxcVec::new_in(ctx);
    if uses_args {
        body.push(make_args_declaration(args_param_name, call_site_arg, ctx));
    }
    body.extend(
        body_stmts
            .iter()
            .map(|stmt| stmt.clone_in_with_semantic_ids(ctx.allocator())),
    );
    if !uses_args {
        return body;
    }
    OxcVec::from_array_in([Statement::new_block_statement(SPAN, body, ctx)], ctx)
}

fn make_args_declaration<'a>(name: &str, value: &Expression<'a>, ctx: &Ctx<'a>) -> Statement<'a> {
    let name = oxc_ast::ast::Ident::from_str_in(name, ctx);
    let id = BindingPattern::new_binding_identifier(SPAN, name, ctx);
    let declarator = VariableDeclarator::new(
        SPAN,
        id,
        None,
        Some(value.clone_in_with_semantic_ids(ctx.allocator())),
        false,
        ctx,
    );
    Statement::new_variable_declaration(SPAN, VariableDeclarationKind::Var, [declarator], false, ctx)
}
