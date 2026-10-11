use oxc_allocator::{CloneIn, GetAllocator};
use oxc_ast::ast::{Argument, BindingPattern, CallExpression, Expression, IdentifierName, Statement};
use oxc_span::SPAN;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::Traverse;
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::rename_guards::is_annex_b_function;
use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;
use super::types::MethodForwarderInfo;

pub struct MethodCallForwarderInliner {
    forwarders: FxHashMap<SymbolId, MethodForwarderInfo>,
    inlined: usize,
}

impl MethodCallForwarderInliner {
    #[must_use]
    pub const fn new(forwarders: FxHashMap<SymbolId, MethodForwarderInfo>) -> Self {
        Self { forwarders, inlined: 0 }
    }

    #[must_use]
    pub const fn inlined(&self) -> usize {
        self.inlined
    }

    fn is_forwarder(&self, symbol: Option<SymbolId>) -> bool {
        symbol.is_some_and(|symbol| self.forwarders.contains_key(&symbol))
    }

    fn try_inline<'a>(&mut self, call: &CallExpression<'a>, ctx: &mut Ctx<'a>) -> Option<Expression<'a>> {
        let Expression::Identifier(ident) = &call.callee else {
            return None;
        };
        let symbol = ctx.scoping().get_reference(ident.reference_id()).symbol_id()?;
        let info = self.forwarders.get(&symbol)?;
        let [recv, arg] = call.arguments.as_slice() else {
            return None;
        };
        let (recv, arg) = (recv.as_expression()?, arg.as_expression()?);
        if self.inlined < 10 {
            eprintln!("[AST/method-fwd] inline {}(a, b) -> a.{}(b)", ident.name, info.method);
        }
        self.inlined += 1;
        let property = IdentifierName::new(SPAN, ctx.allocator().alloc_str(&info.method), ctx);
        let member = Expression::new_static_member_expression(
            SPAN,
            recv.clone_in_with_semantic_ids(ctx.allocator()),
            property,
            false,
            ctx,
        );
        Some(Expression::new_call_expression(
            SPAN,
            member,
            None,
            [Argument::from(arg.clone_in_with_semantic_ids(ctx.allocator()))],
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
        let remove = match stmt {
            Statement::FunctionDeclaration(func) => {
                !is_annex_b_function(ctx, func) && self.is_forwarder(func.id.as_ref().and_then(|id| id.symbol_id.get()))
            }
            Statement::VariableDeclaration(decl) => decl.declarations.iter().all(|d| {
                let BindingPattern::BindingIdentifier(id) = &d.id else {
                    return false;
                };
                matches!(&d.init, Some(Expression::FunctionExpression(_))) && self.is_forwarder(id.symbol_id.get())
            }),
            _ => false,
        };
        if remove {
            *stmt = Statement::new_empty_statement(SPAN, ctx);
        }
    }
}
