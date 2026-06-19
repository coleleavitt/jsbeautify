use oxc_ast::ast::{BindingPattern, Expression, Function, Statement, VariableDeclarator};
use oxc_traverse::Traverse;
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;
use super::types::OperatorProxyInfo;

pub struct OperatorProxyCollector {
    proxies: FxHashMap<String, OperatorProxyInfo>,
}

impl OperatorProxyCollector {
    #[must_use]
    pub fn new() -> Self {
        Self {
            proxies: FxHashMap::default(),
        }
    }

    #[must_use]
    pub fn get_proxies(&self) -> FxHashMap<String, OperatorProxyInfo> {
        self.proxies.clone()
    }

    fn try_extract_proxy(&mut self, func: &Function<'_>) -> Option<(String, OperatorProxyInfo)> {
        let name = func.id.as_ref()?.name.as_str().to_string();
        let info = Self::extract_proxy_info(func, &name)?;
        Some((name, info))
    }

    fn try_extract_proxy_for(&mut self, name: &str, func: &Function<'_>) -> Option<OperatorProxyInfo> {
        Self::extract_proxy_info(func, name)
    }

    fn extract_proxy_info(func: &Function<'_>, name: &str) -> Option<OperatorProxyInfo> {
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

        let Some(Expression::BinaryExpression(binary)) = &ret.argument else {
            return None;
        };

        if func.params.items.len() != 2 {
            return None;
        }

        let param1 = if let BindingPattern::BindingIdentifier(ident) = &func.params.items[0].pattern {
            ident.name.as_str().to_string()
        } else {
            return None;
        };

        let param2 = if let BindingPattern::BindingIdentifier(ident) = &func.params.items[1].pattern {
            ident.name.as_str().to_string()
        } else {
            return None;
        };

        let Expression::Identifier(left_ident) = &binary.left else {
            return None;
        };
        let Expression::Identifier(right_ident) = &binary.right else {
            return None;
        };

        if left_ident.name.as_str() != param1 || right_ident.name.as_str() != param2 {
            return None;
        }

        eprintln!("[AST] Found operator proxy: {name} -> {:?}", binary.operator);

        Some(OperatorProxyInfo {
            operator: binary.operator,
            param1,
            param2,
        })
    }
}

impl Default for OperatorProxyCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for OperatorProxyCollector {
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
}
