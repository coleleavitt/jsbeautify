use oxc_ast::ast::{BindingPattern, CallExpression, Expression, Function, Statement, VariableDeclarator};
use oxc_traverse::Traverse;
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;
use super::types::CallProxyInfo;

pub struct CallProxyCollector {
    proxies: FxHashMap<String, CallProxyInfo>,
    call_counts: FxHashMap<String, usize>,
}

impl CallProxyCollector {
    #[must_use]
    pub fn new() -> Self {
        Self {
            proxies: FxHashMap::default(),
            call_counts: FxHashMap::default(),
        }
    }

    #[must_use]
    pub fn get_single_use_proxies(&self) -> FxHashMap<String, CallProxyInfo> {
        self.proxies
            .iter()
            .filter(|(name, _)| {
                let count = self.call_counts.get(*name).copied().unwrap_or(0);
                count == 1
            })
            .map(|(name, info)| (name.clone(), info.clone()))
            .collect()
    }

    fn try_extract_proxy(&mut self, func: &Function<'_>) -> Option<(String, CallProxyInfo)> {
        let name = func.id.as_ref()?.name.as_str().to_string();
        let info = Self::extract_proxy_info(func, &name)?;
        Some((name, info))
    }

    fn try_extract_proxy_for(&mut self, name: &str, func: &Function<'_>) -> Option<CallProxyInfo> {
        Self::extract_proxy_info(func, name)
    }

    fn extract_proxy_info(func: &Function<'_>, name: &str) -> Option<CallProxyInfo> {
        if func.r#async || func.generator {
            return None;
        }

        let body = func.body.as_ref()?;

        if body.statements.len() != 1 {
            return None;
        }

        let Statement::ReturnStatement(ret) = &body.statements[0] else {
            return None;
        };

        let Some(Expression::CallExpression(call)) = &ret.argument else {
            return None;
        };

        let Expression::Identifier(target_ident) = &call.callee else {
            return None;
        };

        let target_name = target_ident.name.as_str().to_string();

        let params: Vec<String> = func
            .params
            .items
            .iter()
            .filter_map(|p| {
                if let BindingPattern::BindingIdentifier(ident) = &p.pattern {
                    Some(ident.name.as_str().to_string())
                } else {
                    None
                }
            })
            .collect();

        if params.len() != func.params.items.len() {
            return None;
        }

        if call.arguments.len() != params.len() {
            return None;
        }

        for (i, arg) in call.arguments.iter().enumerate() {
            let arg_expr = arg.as_expression()?;
            let Expression::Identifier(arg_ident) = arg_expr else {
                return None;
            };
            if arg_ident.name.as_str() != params[i] {
                return None;
            }
        }

        eprintln!(
            "[AST] Found call proxy: {} -> {} ({} params)",
            name,
            target_name,
            params.len()
        );

        Some(CallProxyInfo { target_name, params })
    }
}

impl Default for CallProxyCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for CallProxyCollector {
    fn enter_function(&mut self, func: &mut Function<'a>, _ctx: &mut Ctx<'a>) {
        if let Some((name, info)) = self.try_extract_proxy(func) {
            self.proxies.insert(name, info);
        }
    }

    fn enter_variable_declarator(&mut self, declarator: &mut VariableDeclarator<'a>, _ctx: &mut Ctx<'a>) {
        let BindingPattern::BindingIdentifier(id) = &declarator.id else {
            return;
        };
        let Some(Expression::FunctionExpression(func)) = &declarator.init else {
            return;
        };
        let name = id.name.as_str().to_string();
        if let Some(info) = self.try_extract_proxy_for(&name, func) {
            self.proxies.insert(name, info);
        }
    }

    fn enter_call_expression(&mut self, call: &mut CallExpression<'a>, _ctx: &mut Ctx<'a>) {
        if let Expression::Identifier(ident) = &call.callee {
            let name = ident.name.as_str().to_string();
            *self.call_counts.entry(name).or_insert(0) += 1;
        }
    }
}
