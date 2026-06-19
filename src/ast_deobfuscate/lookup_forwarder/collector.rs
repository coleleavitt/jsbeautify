use oxc_ast::ast::{BindingPattern, Expression, Function, Statement, VariableDeclarator};
use oxc_traverse::Traverse;
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;
use super::types::LookupForwarderInfo;

pub struct LookupForwarderCollector {
    forwarders: FxHashMap<String, LookupForwarderInfo>,
}

impl LookupForwarderCollector {
    #[must_use]
    pub fn new() -> Self {
        Self {
            forwarders: FxHashMap::default(),
        }
    }

    #[must_use]
    pub fn forwarders(self) -> FxHashMap<String, LookupForwarderInfo> {
        self.forwarders
    }

    pub fn extract_info(func: &Function<'_>) -> Option<LookupForwarderInfo> {
        if func.r#async || func.generator {
            return None;
        }
        if func.params.items.len() != 1 {
            return None;
        }
        let BindingPattern::BindingIdentifier(param) = &func.params.items[0].pattern else {
            return None;
        };
        let body = func.body.as_ref()?;
        if body.statements.len() != 1 {
            return None;
        }
        let Statement::ReturnStatement(ret) = &body.statements[0] else {
            return None;
        };
        let Some(Expression::ComputedMemberExpression(cme)) = &ret.argument else {
            return None;
        };

        let Expression::CallExpression(call) = &cme.object else {
            return None;
        };
        if !call.arguments.is_empty() {
            return None;
        }
        let Expression::Identifier(accessor) = &call.callee else {
            return None;
        };

        let Expression::Identifier(index_ref) = &cme.expression else {
            return None;
        };
        if index_ref.name.as_str() != param.name.as_str() {
            return None;
        }

        Some(LookupForwarderInfo {
            accessor_name: accessor.name.as_str().to_string(),
        })
    }
}

impl Default for LookupForwarderCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for LookupForwarderCollector {
    fn enter_function(&mut self, func: &mut Function<'a>, _ctx: &mut Ctx<'a>) {
        if let Some(id) = &func.id
            && let Some(info) = Self::extract_info(func)
        {
            eprintln!(
                "[AST/lookup-fwd] found  function {}(x) {{ return {}()[x]; }}",
                id.name.as_str(),
                info.accessor_name
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
                "[AST/lookup-fwd] found  var {} = function(x) {{ return {}()[x]; }};",
                id.name.as_str(),
                info.accessor_name
            );
            self.forwarders.insert(id.name.as_str().to_string(), info);
        }
    }
}
