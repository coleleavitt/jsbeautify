use oxc_allocator::{CloneIn, GetAllocator};
use oxc_ast::ast::{BindingPattern, CallExpression, Expression, Statement};
use oxc_span::SPAN;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::Traverse;
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;
use super::types::OperatorProxyInfo;

pub struct OperatorProxyInliner {
    proxies: FxHashMap<SymbolId, OperatorProxyInfo>,
    changed: bool,
}

impl OperatorProxyInliner {
    #[must_use]
    pub const fn new(proxies: FxHashMap<SymbolId, OperatorProxyInfo>) -> Self {
        Self {
            proxies,
            changed: false,
        }
    }

    #[must_use]
    pub const fn has_changed(&self) -> bool {
        self.changed
    }

    fn is_proxy(&self, symbol: Option<SymbolId>) -> bool {
        symbol.is_some_and(|symbol| self.proxies.contains_key(&symbol))
    }

    fn try_inline_call<'a>(&mut self, call: &CallExpression<'a>, ctx: &mut Ctx<'a>) -> Option<Expression<'a>> {
        let Expression::Identifier(ident) = &call.callee else {
            return None;
        };
        let symbol = ctx.scoping().get_reference(ident.reference_id()).symbol_id()?;
        let proxy = self.proxies.get(&symbol)?;
        let [arg1, arg2] = call.arguments.as_slice() else {
            return None;
        };
        let (arg1, arg2) = (arg1.as_expression()?, arg2.as_expression()?);

        eprintln!("[AST] Inlining operator proxy: {} -> {:?}", ident.name, proxy.operator);
        self.changed = true;

        Some(Expression::new_binary_expression(
            SPAN,
            arg1.clone_in_with_semantic_ids(ctx.allocator()),
            proxy.operator,
            arg2.clone_in_with_semantic_ids(ctx.allocator()),
            ctx,
        ))
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for OperatorProxyInliner {
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        if let Expression::CallExpression(call) = expr
            && let Some(inlined) = self.try_inline_call(call, ctx)
        {
            *expr = inlined;
        }
    }

    fn exit_statement(&mut self, stmt: &mut Statement<'a>, ctx: &mut Ctx<'a>) {
        let remove = match stmt {
            Statement::FunctionDeclaration(func) => self.is_proxy(func.id.as_ref().and_then(|id| id.symbol_id.get())),
            Statement::VariableDeclaration(decl) => decl.declarations.iter().all(|d| {
                let BindingPattern::BindingIdentifier(id) = &d.id else {
                    return false;
                };
                matches!(&d.init, Some(Expression::FunctionExpression(_))) && self.is_proxy(id.symbol_id.get())
            }),
            _ => false,
        };
        if remove {
            self.changed = true;
            *stmt = Statement::new_empty_statement(SPAN, ctx);
        }
    }
}
