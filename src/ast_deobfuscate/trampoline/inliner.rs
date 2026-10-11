use oxc_allocator::{ArenaVec, CloneIn, GetAllocator, Vec as OxcVec};
use oxc_ast::ast::{
    Argument, ArrayExpressionElement, AssignmentTarget, BindingPattern, CallExpression, Expression, IdentifierName,
    Statement,
};
use oxc_span::SPAN;
use oxc_traverse::Traverse;
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::rename_guards::{binds_global, is_annex_b_function, references_global};
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
        let dispatcher = ctx.ast.allocator().alloc_str(dispatcher);
        let state = ctx.ast.allocator().alloc_str(state);
        let dispatcher_ident = Expression::new_identifier(SPAN, dispatcher, &ctx.ast);
        let call_member = Expression::new_static_member_expression(
            SPAN,
            dispatcher_ident,
            IdentifierName::new(SPAN, "call", &ctx.ast),
            false,
            &ctx.ast,
        );

        let this_arg = Expression::new_this_expression(SPAN, &ctx.ast);
        let state_arg = Expression::new_identifier(SPAN, state, &ctx.ast);

        let mut array_elems: OxcVec<'a, ArrayExpressionElement<'a>> = ArenaVec::new_in(&ctx.ast);
        for arg in call_args {
            if let Some(e) = arg.as_expression() {
                array_elems.push(ArrayExpressionElement::from(
                    e.clone_in_with_semantic_ids(ctx.ast.allocator()),
                ));
            } else {
                let cloned_arg: Argument<'a> = arg.clone_in_with_semantic_ids(ctx.ast.allocator());
                array_elems.push(cloned_arg.into());
            }
        }
        let args_array = Expression::new_array_expression(SPAN, array_elems, &ctx.ast);

        let mut new_arguments: OxcVec<'a, Argument<'a>> = ArenaVec::new_in(&ctx.ast);
        new_arguments.push(Argument::from(this_arg));
        new_arguments.push(Argument::from(state_arg));
        new_arguments.push(Argument::from(args_array));

        Expression::new_call_expression_with_pure(SPAN, call_member, None, new_arguments, false, false, &ctx.ast)
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
            && !binds_global(ctx, id)
            && !is_annex_b_function(ctx, func)
        {
            eprintln!("[AST/tramp] remove function {}", id.name.as_str());
            *stmt = Statement::new_empty_statement(SPAN, &ctx.ast);
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
                    self.trampolines.contains_key(id.name.as_str()) && !binds_global(ctx, id)
                });
            if all_are_trampolines {
                eprintln!("[AST/tramp] remove var declaration");
                *stmt = Statement::new_empty_statement(SPAN, &ctx.ast);
                return;
            }
        }
        if let Statement::ExpressionStatement(es) = stmt
            && let Expression::AssignmentExpression(assign) = &es.expression
            && let AssignmentTarget::AssignmentTargetIdentifier(target) = &assign.left
            && matches!(&assign.right, Expression::FunctionExpression(_))
            && self.trampolines.contains_key(target.name.as_str())
            && !references_global(ctx, target)
        {
            eprintln!("[AST/tramp] remove {} = function()", target.name.as_str());
            *stmt = Statement::new_empty_statement(SPAN, &ctx.ast);
        }
    }
}
