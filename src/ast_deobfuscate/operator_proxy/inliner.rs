use oxc_allocator::{CloneIn, GetAllocator};
use oxc_ast::ast::{BindingPattern, CallExpression, Expression, Statement};
use oxc_span::SPAN;
use oxc_traverse::Traverse;
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::rename_guards::{binds_global, is_annex_b_function};
use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;
use super::types::OperatorProxyInfo;

pub struct OperatorProxyInliner {
    proxies: FxHashMap<String, OperatorProxyInfo>,
    changed: bool,
}

impl OperatorProxyInliner {
    #[must_use]
    pub const fn new(proxies: FxHashMap<String, OperatorProxyInfo>) -> Self {
        Self {
            proxies,
            changed: false,
        }
    }

    #[must_use]
    pub const fn has_changed(&self) -> bool {
        self.changed
    }

    fn try_inline_call<'a>(&mut self, call: &CallExpression<'a>, ctx: &mut Ctx<'a>) -> Option<Expression<'a>> {
        let name = if let Expression::Identifier(ident) = &call.callee {
            ident.name.as_str()
        } else {
            return None;
        };

        let proxy = self.proxies.get(name)?;

        if call.arguments.len() != 2 {
            return None;
        }

        let arg1 = call.arguments[0].as_expression()?;
        let arg2 = call.arguments[1].as_expression()?;

        eprintln!("[AST] Inlining operator proxy: {} -> {:?}", name, proxy.operator);
        self.changed = true;

        Some(Expression::new_binary_expression(
            SPAN,
            Self::clone_expression(arg1, ctx),
            proxy.operator,
            Self::clone_expression(arg2, ctx),
            ctx,
        ))
    }

    fn clone_expression<'a>(expr: &Expression<'a>, ctx: &mut Ctx<'a>) -> Expression<'a> {
        expr.clone_in_with_semantic_ids(ctx.allocator())
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
        if let Statement::FunctionDeclaration(func) = stmt
            && let Some(id) = &func.id
        {
            let name = id.name.as_str();
            if self.proxies.contains_key(name) && !binds_global(ctx, id) && !is_annex_b_function(ctx, func) {
                eprintln!("[AST] Removing operator proxy function: {name}");
                self.changed = true;
                *stmt = Statement::new_empty_statement(SPAN, ctx);
                return;
            }
        }

        if let Statement::VariableDeclaration(decl) = stmt {
            let all_are_proxies = !decl.declarations.is_empty()
                && decl.declarations.iter().all(|d| {
                    let BindingPattern::BindingIdentifier(id) = &d.id else {
                        return false;
                    };
                    let Some(Expression::FunctionExpression(_)) = &d.init else {
                        return false;
                    };
                    self.proxies.contains_key(id.name.as_str()) && !binds_global(ctx, id)
                });
            if all_are_proxies {
                let names: Vec<String> = decl
                    .declarations
                    .iter()
                    .filter_map(|d| match &d.id {
                        BindingPattern::BindingIdentifier(id) => Some(id.name.as_str().to_string()),
                        _ => None,
                    })
                    .collect();
                eprintln!(
                    "[AST] Removing operator proxy var declaration: var {};",
                    names.join(", ")
                );
                self.changed = true;
                *stmt = Statement::new_empty_statement(SPAN, ctx);
            }
        }
    }
}
