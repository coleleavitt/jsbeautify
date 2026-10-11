use oxc_allocator::{CloneIn, GetAllocator};
use oxc_ast::ast::{BindingPattern, CallExpression, Expression, Statement};
use oxc_span::SPAN;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::Traverse;
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::rename_guards::is_annex_b_function;
use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;
use super::types::LookupForwarderInfo;

pub struct LookupForwarderInliner {
    forwarders: FxHashMap<SymbolId, LookupForwarderInfo>,
    inlined: usize,
}

impl LookupForwarderInliner {
    #[must_use]
    pub const fn new(forwarders: FxHashMap<SymbolId, LookupForwarderInfo>) -> Self {
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
        let [arg] = call.arguments.as_slice() else {
            return None;
        };
        let arg = arg.as_expression()?;
        if self.inlined < 10 {
            eprintln!(
                "[AST/lookup-fwd] inline {}(x) -> {}()[x]",
                ident.name, info.accessor_name
            );
        }
        self.inlined += 1;
        let accessor = Expression::new_identifier(SPAN, ctx.allocator().alloc_str(&info.accessor_name), ctx);
        let accessor_call = Expression::new_call_expression(SPAN, accessor, None, [], false, ctx);
        let index = arg.clone_in_with_semantic_ids(ctx.allocator());
        Some(Expression::new_computed_member_expression(
            SPAN,
            accessor_call,
            index,
            false,
            ctx,
        ))
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for LookupForwarderInliner {
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
