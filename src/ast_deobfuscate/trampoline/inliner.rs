use oxc_allocator::{ArenaVec, CloneIn, GetAllocator, Vec as OxcVec};
use oxc_ast::ast::{
    Argument, ArrayExpressionElement, AssignmentTarget, BindingPattern, CallExpression, Expression, Statement,
};
use oxc_span::SPAN;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::Traverse;
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::rename_guards::is_annex_b_function;
use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;
use super::types::TrampolineInfo;

pub struct TrampolineInliner {
    trampolines: FxHashMap<SymbolId, TrampolineInfo>,
    inlined: usize,
}

impl TrampolineInliner {
    #[must_use]
    pub const fn new(trampolines: FxHashMap<SymbolId, TrampolineInfo>) -> Self {
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
        info: &TrampolineInfo,
        call_args: &OxcVec<'a, Argument<'a>>,
        ctx: &Ctx<'a>,
    ) -> Expression<'a> {
        let dispatcher = ctx.ast.allocator().alloc_str(&info.dispatcher);
        let state = ctx.ast.allocator().alloc_str(&info.state);
        let dispatcher_ident = Expression::new_identifier(SPAN, dispatcher, &ctx.ast);
        let state_arg = Expression::new_identifier(SPAN, state, &ctx.ast);

        let mut array_elems: OxcVec<'a, ArrayExpressionElement<'a>> = ArenaVec::new_in(&ctx.ast);
        for arg in call_args {
            let cloned_arg: Argument<'a> = arg.clone_in_with_semantic_ids(ctx.ast.allocator());
            array_elems.push(cloned_arg.into());
        }
        let args_array = Expression::new_array_expression(SPAN, array_elems, &ctx.ast);

        let mut new_arguments: OxcVec<'a, Argument<'a>> = ArenaVec::new_in(&ctx.ast);
        new_arguments.push(Argument::from(state_arg));
        new_arguments.push(Argument::from(args_array));

        Expression::new_call_expression(SPAN, dispatcher_ident, None, new_arguments, false, &ctx.ast)
    }

    fn trampoline_of(&self, symbol: Option<SymbolId>) -> Option<&TrampolineInfo> {
        self.trampolines.get(&symbol?)
    }

    fn try_inline<'a>(&mut self, call: &CallExpression<'a>, ctx: &mut Ctx<'a>) -> Option<Expression<'a>> {
        let Expression::Identifier(ident) = &call.callee else {
            return None;
        };
        let symbol = ctx.scoping().get_reference(ident.reference_id()).symbol_id();
        let info = self.trampoline_of(symbol)?;
        if self.inlined < 10 {
            eprintln!(
                "[AST/tramp] inline {}({} args) -> {}({}, [...])",
                ident.name.as_str(),
                call.arguments.len(),
                info.dispatcher,
                info.state
            );
        }
        let inlined = Self::make_call_expr(info, &call.arguments, ctx);
        self.inlined += 1;
        Some(inlined)
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
        let remove = match stmt {
            Statement::FunctionDeclaration(func) => {
                !is_annex_b_function(ctx, func)
                    && self
                        .trampoline_of(func.id.as_ref().and_then(|id| id.symbol_id.get()))
                        .is_some()
            }
            Statement::VariableDeclaration(decl) => decl.declarations.iter().all(|d| {
                let BindingPattern::BindingIdentifier(id) = &d.id else {
                    return false;
                };
                matches!(&d.init, Some(Expression::FunctionExpression(_)))
                    && self.trampoline_of(id.symbol_id.get()).is_some()
            }),
            Statement::ExpressionStatement(es) => {
                if let Expression::AssignmentExpression(assign) = &es.expression
                    && let AssignmentTarget::AssignmentTargetIdentifier(target) = &assign.left
                    && matches!(&assign.right, Expression::FunctionExpression(_))
                {
                    let symbol = ctx.scoping().get_reference(target.reference_id()).symbol_id();
                    self.trampoline_of(symbol).is_some()
                } else {
                    false
                }
            }
            _ => false,
        };
        if remove {
            *stmt = Statement::new_empty_statement(SPAN, &ctx.ast);
        }
    }
}
