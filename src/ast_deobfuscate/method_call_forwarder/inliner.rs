use oxc_allocator::{CloneIn, GetAllocator};
use oxc_ast::ast::{Argument, BindingPattern, CallExpression, Expression, IdentifierName, Statement};
use oxc_span::SPAN;
use oxc_traverse::Traverse;
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::rename_guards::{binds_global, is_annex_b_function};
use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;
use super::types::MethodForwarderInfo;

pub struct MethodCallForwarderInliner {
    forwarders: FxHashMap<String, MethodForwarderInfo>,
    inlined: usize,
}

impl MethodCallForwarderInliner {
    #[must_use]
    pub const fn new(forwarders: FxHashMap<String, MethodForwarderInfo>) -> Self {
        Self { forwarders, inlined: 0 }
    }

    #[must_use]
    pub const fn inlined(&self) -> usize {
        self.inlined
    }

    fn try_inline<'a>(&mut self, call: &CallExpression<'a>, ctx: &mut Ctx<'a>) -> Option<Expression<'a>> {
        let Expression::Identifier(ident) = &call.callee else {
            return None;
        };
        let info = self.forwarders.get(ident.name.as_str())?;
        if call.arguments.len() != 2 {
            return None;
        }
        let recv = call.arguments[0].as_expression()?;
        let arg = call.arguments[1].as_expression()?;
        if self.inlined < 10 {
            eprintln!(
                "[AST/method-fwd] inline {}(a, b) -> a.{}(b)",
                ident.name.as_str(),
                info.method
            );
        }
        self.inlined += 1;

        let recv_cloned = recv.clone_in_with_semantic_ids(ctx.ast.allocator());
        let arg_cloned = arg.clone_in_with_semantic_ids(ctx.ast.allocator());
        let property = IdentifierName::new(SPAN, ctx.allocator().alloc_str(&info.method), ctx);
        let member = Expression::new_static_member_expression(SPAN, recv_cloned, property, false, ctx);
        Some(Expression::new_call_expression(
            SPAN,
            member,
            None,
            [Argument::from(arg_cloned)],
            false,
            ctx,
        ))
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for MethodCallForwarderInliner {
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
            && self.forwarders.contains_key(id.name.as_str())
            && !binds_global(ctx, id)
            && !is_annex_b_function(ctx, func)
        {
            eprintln!("[AST/method-fwd] remove function {}", id.name.as_str());
            *stmt = Statement::new_empty_statement(SPAN, ctx);
            return;
        }
        if let Statement::VariableDeclaration(decl) = stmt {
            let all_are_forwarders = !decl.declarations.is_empty()
                && decl.declarations.iter().all(|d| {
                    let BindingPattern::BindingIdentifier(id) = &d.id else {
                        return false;
                    };
                    let Some(Expression::FunctionExpression(_)) = &d.init else {
                        return false;
                    };
                    self.forwarders.contains_key(id.name.as_str()) && !binds_global(ctx, id)
                });
            if all_are_forwarders {
                eprintln!("[AST/method-fwd] remove var declaration");
                *stmt = Statement::new_empty_statement(SPAN, ctx);
            }
        }
    }
}
