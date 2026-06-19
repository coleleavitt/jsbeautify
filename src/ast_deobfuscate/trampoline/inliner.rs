use oxc_allocator::{CloneIn, Vec as OxcVec};
use oxc_ast::ast::{
    Argument, ArrayExpression, ArrayExpressionElement, AssignmentTarget, BindingPattern, CallExpression,
    EmptyStatement, Expression, IdentifierName, IdentifierReference, Statement, StaticMemberExpression, ThisExpression,
};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use oxc_traverse::Traverse;
use rustc_hash::FxHashMap;
use std::cell::Cell;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;
use super::types::TrampolineInfo;

pub struct TrampolineInliner {
    trampolines: FxHashMap<String, TrampolineInfo>,
    inlined: usize,
}

impl TrampolineInliner {
    #[must_use]
    pub const fn new(trampolines: FxHashMap<String, TrampolineInfo>) -> Self {
        Self {
            trampolines,
            inlined: 0,
        }
    }

    #[must_use]
    pub const fn inlined(&self) -> usize {
        self.inlined
    }

    fn make_call_expr<'a>(
        dispatcher: &str,
        state: &str,
        call_args: &OxcVec<'a, Argument<'a>>,
        ctx: &Ctx<'a>,
    ) -> Expression<'a> {
        let dispatcher_ident = Expression::Identifier(ctx.ast.alloc(IdentifierReference {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            name: ctx.ast.ident(dispatcher),
            reference_id: Cell::default(),
        }));
        let call_member = Expression::StaticMemberExpression(ctx.ast.alloc(StaticMemberExpression {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            object: dispatcher_ident,
            property: IdentifierName {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                name: ctx.ast.ident("call"),
            },
            optional: false,
        }));

        let this_arg = Expression::ThisExpression(ctx.ast.alloc(ThisExpression {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
        }));
        let state_arg = Expression::Identifier(ctx.ast.alloc(IdentifierReference {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            name: ctx.ast.ident(state),
            reference_id: Cell::default(),
        }));

        let mut array_elems: OxcVec<'a, ArrayExpressionElement<'a>> = ctx.ast.vec();
        for arg in call_args {
            if let Some(e) = arg.as_expression() {
                array_elems.push(ArrayExpressionElement::from(
                    e.clone_in_with_semantic_ids(ctx.ast.allocator),
                ));
            } else {
                let cloned_arg: Argument<'a> = arg.clone_in_with_semantic_ids(ctx.ast.allocator);
                array_elems.push(cloned_arg.into());
            }
        }
        let args_array = Expression::ArrayExpression(ctx.ast.alloc(ArrayExpression {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            elements: array_elems,
        }));

        let mut new_arguments: OxcVec<'a, Argument<'a>> = ctx.ast.vec();
        new_arguments.push(Argument::from(this_arg));
        new_arguments.push(Argument::from(state_arg));
        new_arguments.push(Argument::from(args_array));

        Expression::CallExpression(ctx.ast.alloc(CallExpression {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            callee: call_member,
            arguments: new_arguments,
            optional: false,
            type_arguments: None,
            pure: false,
        }))
    }

    fn try_inline<'a>(&mut self, call: &CallExpression<'a>, ctx: &mut Ctx<'a>) -> Option<Expression<'a>> {
        let Expression::Identifier(ident) = &call.callee else {
            return None;
        };
        let info = self.trampolines.get(ident.name.as_str())?;
        if self.inlined < 10 {
            eprintln!(
                "[AST/tramp] inline {}({} args) -> {}.call(this, {}, [...])",
                ident.name.as_str(),
                call.arguments.len(),
                info.dispatcher,
                info.state
            );
        }
        self.inlined += 1;
        Some(Self::make_call_expr(
            &info.dispatcher,
            &info.state,
            &call.arguments,
            ctx,
        ))
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for TrampolineInliner {
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        if let Expression::CallExpression(call) = expr
            && let Some(inlined) = self.try_inline(call, ctx)
        {
            *expr = inlined;
        }
    }

    fn exit_statement(&mut self, stmt: &mut Statement<'a>, ctx: &mut Ctx<'a>) {
        if let Statement::FunctionDeclaration(func) = stmt
            && let Some(id) = &func.id
            && self.trampolines.contains_key(id.name.as_str())
        {
            eprintln!("[AST/tramp] remove function {}", id.name.as_str());
            *stmt = Statement::EmptyStatement(ctx.ast.alloc(EmptyStatement {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
            }));
            return;
        }
        if let Statement::VariableDeclaration(decl) = stmt {
            let all_are_trampolines = !decl.declarations.is_empty()
                && decl.declarations.iter().all(|d| {
                    let BindingPattern::BindingIdentifier(id) = &d.id else {
                        return false;
                    };
                    let Some(Expression::FunctionExpression(_)) = &d.init else {
                        return false;
                    };
                    self.trampolines.contains_key(id.name.as_str())
                });
            if all_are_trampolines {
                eprintln!("[AST/tramp] remove var declaration");
                *stmt = Statement::EmptyStatement(ctx.ast.alloc(EmptyStatement {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                }));
                return;
            }
        }
        if let Statement::ExpressionStatement(es) = stmt
            && let Expression::AssignmentExpression(assign) = &es.expression
            && let AssignmentTarget::AssignmentTargetIdentifier(target) = &assign.left
            && matches!(&assign.right, Expression::FunctionExpression(_))
            && self.trampolines.contains_key(target.name.as_str())
        {
            eprintln!("[AST/tramp] remove {} = function()", target.name.as_str());
            *stmt = Statement::EmptyStatement(ctx.ast.alloc(EmptyStatement {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
            }));
        }
    }
}
