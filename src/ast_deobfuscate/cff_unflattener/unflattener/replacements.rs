use oxc_allocator::{CloneIn, GetAllocator, Vec as OxcVec};
use oxc_ast::ast::{
    ArrowFunctionBody, BindingPattern, Expression, FormalParameterKind, FormalParameters, FunctionBody, Statement,
    VariableDeclarationKind, VariableDeclarator,
};
use oxc_span::SPAN;

use crate::ast_deobfuscate::state::DeobfuscateState;

type Ctx<'a> = oxc_traverse::TraverseCtx<'a, DeobfuscateState>;

/// `(() => { var ARGS = args; …body })()`; an arrow keeps the call site's
/// `this`, which the resolver only allows when the body ignores it or the call
/// forwarded it.
pub(super) fn make_iife<'a>(
    args_param_name: &str,
    body_stmts: &OxcVec<'a, Statement<'a>>,
    call_site_arg: Option<&Expression<'a>>,
    ctx: &Ctx<'a>,
) -> Expression<'a> {
    let mut stmts = OxcVec::new_in(ctx);
    if let Some(value) = call_site_arg {
        stmts.push(make_args_declaration(args_param_name, value, ctx));
    }
    stmts.extend(clone_statements(body_stmts, ctx));

    let params = FormalParameters::boxed(SPAN, FormalParameterKind::ArrowFormalParameters, [], None, ctx);
    let body = ArrowFunctionBody::FunctionBody(FunctionBody::boxed(SPAN, [], stmts, ctx));
    let callee = Expression::new_arrow_function_expression(SPAN, false, None, params, None, body, ctx);
    Expression::new_call_expression(SPAN, callee, None, [], false, ctx)
}

pub(super) fn clone_statements<'a>(
    body_stmts: &OxcVec<'a, Statement<'a>>,
    ctx: &Ctx<'a>,
) -> impl Iterator<Item = Statement<'a>> {
    body_stmts
        .iter()
        .map(|stmt| stmt.clone_in_with_semantic_ids(ctx.allocator()))
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
