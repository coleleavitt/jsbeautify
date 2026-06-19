use oxc_ast::ast::{BindingPattern, Expression, Function, Statement, VariableDeclarator};
use oxc_traverse::Traverse;
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;
use super::types::MethodForwarderInfo;

pub struct MethodCallForwarderCollector {
    forwarders: FxHashMap<String, MethodForwarderInfo>,
}

impl MethodCallForwarderCollector {
    #[must_use]
    pub fn new() -> Self {
        Self {
            forwarders: FxHashMap::default(),
        }
    }

    #[must_use]
    pub fn into_forwarders(self) -> FxHashMap<String, MethodForwarderInfo> {
        self.forwarders
    }

    fn extract_info(func: &Function<'_>) -> Option<MethodForwarderInfo> {
        if func.r#async || func.generator {
            return None;
        }
        if func.params.items.len() != 2 {
            return None;
        }
        let BindingPattern::BindingIdentifier(p0) = &func.params.items[0].pattern else {
            return None;
        };
        let BindingPattern::BindingIdentifier(p1) = &func.params.items[1].pattern else {
            return None;
        };
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
        let Expression::StaticMemberExpression(sme) = &call.callee else {
            return None;
        };
        let Expression::Identifier(receiver) = &sme.object else {
            return None;
        };
        if receiver.name.as_str() != p0.name.as_str() {
            return None;
        }
        if call.arguments.len() != 1 {
            return None;
        }
        let Some(Expression::Identifier(arg0)) = call.arguments[0].as_expression() else {
            return None;
        };
        if arg0.name.as_str() != p1.name.as_str() {
            return None;
        }
        Some(MethodForwarderInfo {
            method: sme.property.name.as_str().to_string(),
        })
    }
}

impl Default for MethodCallForwarderCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for MethodCallForwarderCollector {
    fn enter_function(&mut self, func: &mut Function<'a>, _ctx: &mut Ctx<'a>) {
        if let Some(id) = &func.id
            && let Some(info) = Self::extract_info(func)
        {
            eprintln!(
                "[AST/method-fwd] found  function {}(a, b) {{ return a.{}(b); }}",
                id.name.as_str(),
                info.method
            );
            self.forwarders.insert(id.name.as_str().to_string(), info);
        }
    }

    fn enter_variable_declarator(&mut self, declarator: &mut VariableDeclarator<'a>, _ctx: &mut Ctx<'a>) {
        let BindingPattern::BindingIdentifier(id) = &declarator.id else {
            return;
        };
        let Some(Expression::FunctionExpression(func)) = &declarator.init else {
            return;
        };
        if let Some(info) = Self::extract_info(func) {
            eprintln!(
                "[AST/method-fwd] found  var {} = function(a, b) {{ return a.{}(b); }};",
                id.name.as_str(),
                info.method
            );
            self.forwarders.insert(id.name.as_str().to_string(), info);
        }
    }
}
