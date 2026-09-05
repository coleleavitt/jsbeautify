use oxc_allocator::{CloneIn, GetAllocator, Vec as OxcVec};
use oxc_ast::ast::{Argument, BindingPattern, CallExpression, Expression, Statement};
use oxc_span::SPAN;
use oxc_traverse::Traverse;
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;
use super::types::CallProxyInfo;

pub struct CallProxyInliner {
    proxies: FxHashMap<String, CallProxyInfo>,
    changed: bool,
}

impl CallProxyInliner {
    #[must_use]
    pub const fn new(proxies: FxHashMap<String, CallProxyInfo>) -> Self {
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

        if call.arguments.len() != proxy.params.len() {
            return None;
        }

        eprintln!("[AST] Inlining call proxy: {} -> {}", name, proxy.target_name);
        self.changed = true;

        let mut arguments = OxcVec::new_in(ctx);
        for arg in &call.arguments {
            if let Some(expr) = arg.as_expression() {
                arguments.push(Argument::from(Self::clone_expression(expr, ctx)));
            }
        }
        let target_name = oxc_ast::ast::Ident::from_str_in(&proxy.target_name, ctx);
        let callee = Expression::new_identifier(SPAN, target_name, ctx);
        Some(Expression::new_call_expression(
            SPAN, callee, None, arguments, false, ctx,
        ))
    }

    fn clone_expression<'a>(expr: &Expression<'a>, ctx: &mut Ctx<'a>) -> Expression<'a> {
        expr.clone_in_with_semantic_ids(ctx.allocator())
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for CallProxyInliner {
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
            if self.proxies.contains_key(name) {
                eprintln!("[AST] Removing call proxy function: {name}");
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
                    self.proxies.contains_key(id.name.as_str())
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
                eprintln!("[AST] Removing call proxy var declaration: var {};", names.join(", "));
                self.changed = true;
                *stmt = Statement::new_empty_statement(SPAN, ctx);
            }
        }
    }
}
