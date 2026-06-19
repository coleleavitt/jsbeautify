use oxc_ast::ast::{AssignmentTarget, BindingPattern, Expression, Function, Statement, VariableDeclarator};
use oxc_traverse::Traverse;
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;
use super::types::TrampolineInfo;

pub struct TrampolineCollector {
    trampolines: FxHashMap<String, TrampolineInfo>,
}

impl TrampolineCollector {
    #[must_use]
    pub fn new() -> Self {
        Self {
            trampolines: FxHashMap::default(),
        }
    }

    #[must_use]
    pub fn into_trampolines(self) -> FxHashMap<String, TrampolineInfo> {
        self.trampolines
    }

    fn extract_info(func: &Function<'_>) -> Option<TrampolineInfo> {
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
        let Expression::StaticMemberExpression(sme) = &call.callee else {
            return None;
        };
        if sme.property.name.as_str() != "apply" {
            return None;
        }
        let Expression::Identifier(dispatcher) = &sme.object else {
            return None;
        };
        if call.arguments.len() != 2 {
            return None;
        }
        let first = call.arguments[0].as_expression()?;
        if !matches!(first, Expression::ThisExpression(_)) {
            return None;
        }
        let Some(Expression::ArrayExpression(arr)) = call.arguments[1].as_expression() else {
            return None;
        };
        if arr.elements.len() != 2 {
            return None;
        }
        let Some(Expression::Identifier(state)) = arr.elements[0].as_expression() else {
            return None;
        };
        let Some(Expression::Identifier(second)) = arr.elements[1].as_expression() else {
            return None;
        };
        if second.name.as_str() != "arguments" {
            return None;
        }
        Some(TrampolineInfo {
            dispatcher: dispatcher.name.as_str().to_string(),
            state: state.name.as_str().to_string(),
        })
    }
}

impl Default for TrampolineCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for TrampolineCollector {
    fn enter_function(&mut self, func: &mut Function<'a>, _ctx: &mut Ctx<'a>) {
        if let Some(id) = &func.id
            && let Some(info) = Self::extract_info(func)
        {
            eprintln!(
                "[AST/tramp] found  function {}() -> {}.apply(this, [{}, arguments])",
                id.name.as_str(),
                info.dispatcher,
                info.state
            );
            self.trampolines.insert(id.name.as_str().to_string(), info);
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
                "[AST/tramp] found  var {} = function() -> {}.apply(this, [{}, arguments])",
                id.name.as_str(),
                info.dispatcher,
                info.state
            );
            self.trampolines.insert(id.name.as_str().to_string(), info);
        }
    }

    fn enter_expression(&mut self, expr: &mut Expression<'a>, _ctx: &mut Ctx<'a>) {
        if let Expression::AssignmentExpression(assign) = expr
            && let AssignmentTarget::AssignmentTargetIdentifier(target) = &assign.left
            && let Expression::FunctionExpression(func) = &assign.right
            && let Some(info) = Self::extract_info(func)
        {
            let name = target.name.as_str().to_string();
            eprintln!(
                "[AST/tramp] found  {} = function() -> {}.apply(this, [{}, arguments])",
                name, info.dispatcher, info.state
            );
            self.trampolines.insert(name, info);
        }
    }
}
