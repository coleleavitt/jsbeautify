use oxc_allocator::{CloneIn, Vec as OxcVec};
use oxc_ast::ast::{
    BindingIdentifier, BindingPattern, BlockStatement, CallExpression, Expression, FormalParameterKind,
    FormalParameters, Function, FunctionBody, FunctionType, Statement, VariableDeclaration, VariableDeclarationKind,
    VariableDeclarator,
};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use std::cell::Cell;

use super::super::case_bodies::body_references_param;
use crate::ast_deobfuscate::state::DeobfuscateState;

type Ctx<'a> = oxc_traverse::TraverseCtx<'a, DeobfuscateState>;

pub(super) fn make_iife<'a>(
    args_param_name: &str,
    body_stmts: &OxcVec<'a, Statement<'a>>,
    call_site_arg: &Expression<'a>,
    ctx: &Ctx<'a>,
) -> Expression<'a> {
    let alloc = ctx.ast.allocator;
    let uses_args = body_references_param(body_stmts.as_slice(), args_param_name);

    let mut stmts: OxcVec<'a, Statement<'a>> = ctx.ast.vec();

    if uses_args {
        let binding_id = BindingIdentifier {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            name: ctx.ast.ident(args_param_name),
            symbol_id: Cell::default(),
        };
        let declarator = VariableDeclarator {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            kind: VariableDeclarationKind::Var,
            id: BindingPattern::BindingIdentifier(ctx.ast.alloc(binding_id)),
            type_annotation: None,
            init: Some(call_site_arg.clone_in_with_semantic_ids(alloc)),
            definite: false,
        };
        let mut declarations = ctx.ast.vec();
        declarations.push(declarator);
        stmts.push(Statement::VariableDeclaration(ctx.ast.alloc(VariableDeclaration {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            kind: VariableDeclarationKind::Var,
            declarations,
            declare: false,
        })));
    }

    for s in body_stmts {
        stmts.push(s.clone_in_with_semantic_ids(alloc));
    }

    let fn_body = FunctionBody {
        node_id: Cell::new(NodeId::DUMMY),
        span: SPAN,
        directives: ctx.ast.vec(),
        statements: stmts,
    };

    let params = FormalParameters {
        node_id: Cell::new(NodeId::DUMMY),
        span: SPAN,
        kind: FormalParameterKind::FormalParameter,
        items: ctx.ast.vec(),
        rest: None,
    };

    let func = Function {
        node_id: Cell::new(NodeId::DUMMY),
        span: SPAN,
        r#type: FunctionType::FunctionExpression,
        id: None,
        generator: false,
        r#async: false,
        declare: false,
        type_parameters: None,
        this_param: None,
        params: ctx.ast.alloc(params),
        return_type: None,
        body: Some(ctx.ast.alloc(fn_body)),
        scope_id: Cell::new(None),
        pure: false,
        pife: false,
    };
    let callee = Expression::FunctionExpression(ctx.ast.alloc(func));

    Expression::CallExpression(ctx.ast.alloc(CallExpression {
        node_id: Cell::new(NodeId::DUMMY),
        span: SPAN,
        callee,
        arguments: ctx.ast.vec(),
        optional: false,
        type_arguments: None,
        pure: false,
    }))
}

pub(super) fn make_inline_stmts<'a>(
    args_param_name: &str,
    body_stmts: &OxcVec<'a, Statement<'a>>,
    call_site_arg: &Expression<'a>,
    ctx: &Ctx<'a>,
) -> OxcVec<'a, Statement<'a>> {
    let alloc = ctx.ast.allocator;
    let uses_args = body_references_param(body_stmts.as_slice(), args_param_name);

    if !uses_args {
        let mut stmts: OxcVec<'a, Statement<'a>> = ctx.ast.vec();
        for s in body_stmts {
            stmts.push(s.clone_in_with_semantic_ids(alloc));
        }
        return stmts;
    }

    let mut block_body: OxcVec<'a, Statement<'a>> = ctx.ast.vec();

    let binding_id = BindingIdentifier {
        node_id: Cell::new(NodeId::DUMMY),
        span: SPAN,
        name: ctx.ast.ident(args_param_name),
        symbol_id: Cell::default(),
    };
    let declarator = VariableDeclarator {
        node_id: Cell::new(NodeId::DUMMY),
        span: SPAN,
        kind: VariableDeclarationKind::Var,
        id: BindingPattern::BindingIdentifier(ctx.ast.alloc(binding_id)),
        type_annotation: None,
        init: Some(call_site_arg.clone_in_with_semantic_ids(alloc)),
        definite: false,
    };
    let mut declarations = ctx.ast.vec();
    declarations.push(declarator);
    block_body.push(Statement::VariableDeclaration(ctx.ast.alloc(VariableDeclaration {
        node_id: Cell::new(NodeId::DUMMY),
        span: SPAN,
        kind: VariableDeclarationKind::Var,
        declarations,
        declare: false,
    })));

    for s in body_stmts {
        block_body.push(s.clone_in_with_semantic_ids(alloc));
    }

    let block = Statement::BlockStatement(ctx.ast.alloc(BlockStatement {
        node_id: Cell::new(NodeId::DUMMY),
        span: SPAN,
        body: block_body,
        scope_id: Cell::new(None),
    }));

    let mut result: OxcVec<'a, Statement<'a>> = ctx.ast.vec();
    result.push(block);
    result
}
