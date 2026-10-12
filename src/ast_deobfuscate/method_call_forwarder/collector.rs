use oxc_ast::ast::{BindingPattern, Expression, Function, Program, Statement, VariableDeclarator};
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::Traverse;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::call_sites::plain_call_arities;
use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;
use super::types::MethodForwarderInfo;

/// Finds `function f(a, b) { return a.m(b); }` bindings whose every reference
/// is a two-argument call.
#[derive(Default)]
pub struct MethodCallForwarderCollector {
    binary_calls_only: FxHashSet<SymbolId>,
    forwarders: FxHashMap<SymbolId, MethodForwarderInfo>,
}

impl MethodCallForwarderCollector {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn into_forwarders(self) -> FxHashMap<SymbolId, MethodForwarderInfo> {
        self.forwarders
    }

    fn extract_info(func: &Function<'_>) -> Option<MethodForwarderInfo> {
        if func.r#async
            || func.generator
            || func.params.rest.is_some()
            || func.params.items.iter().any(|param| param.initializer.is_some())
        {
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

    fn add(&mut self, symbol: Option<SymbolId>, func: &Function<'_>) {
        if let Some(symbol) = symbol
            && self.binary_calls_only.contains(&symbol)
            && let Some(info) = Self::extract_info(func)
        {
            deob_trace!("[AST/method-fwd] found forwarder to a.{}(b)", info.method);
            self.forwarders.insert(symbol, info);
        }
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for MethodCallForwarderCollector {
    fn enter_program(&mut self, program: &mut Program<'a>, ctx: &mut Ctx<'a>) {
        self.binary_calls_only = plain_call_arities(program, ctx.scoping())
            .into_iter()
            .filter(|(_, arities)| arities.len() == 1 && arities.contains(&2))
            .map(|(symbol, _)| symbol)
            .collect();
    }

    fn enter_function(&mut self, func: &mut Function<'a>, _ctx: &mut Ctx<'a>) {
        if func.is_declaration() {
            self.add(func.id.as_ref().and_then(|id| id.symbol_id.get()), func);
        }
    }

    fn enter_variable_declarator(&mut self, declarator: &mut VariableDeclarator<'a>, _ctx: &mut Ctx<'a>) {
        if let BindingPattern::BindingIdentifier(id) = &declarator.id
            && let Some(Expression::FunctionExpression(func)) = &declarator.init
        {
            self.add(id.symbol_id.get(), func);
        }
    }
}
