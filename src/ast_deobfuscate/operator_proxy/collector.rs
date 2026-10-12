use oxc_ast::ast::{BindingPattern, Expression, Function, Program, Statement, VariableDeclarator};
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::Traverse;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::call_sites::plain_call_arities;
use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;
use super::types::OperatorProxyInfo;

/// Finds `function f(a, b) { return a OP b; }` bindings whose every reference
/// is a two-argument call.
#[derive(Default)]
pub struct OperatorProxyCollector {
    binary_calls_only: FxHashSet<SymbolId>,
    proxies: FxHashMap<SymbolId, OperatorProxyInfo>,
}

impl OperatorProxyCollector {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn get_proxies(&self) -> FxHashMap<SymbolId, OperatorProxyInfo> {
        self.proxies.clone()
    }

    fn extract_proxy_info(func: &Function<'_>) -> Option<OperatorProxyInfo> {
        if func.r#async || func.generator || func.params.rest.is_some() {
            return None;
        }
        let body = func.body.as_ref()?;
        let [Statement::ReturnStatement(ret)] = body.statements.as_slice() else {
            return None;
        };
        let Some(Expression::BinaryExpression(binary)) = &ret.argument else {
            return None;
        };
        let [first, second] = func.params.items.as_slice() else {
            return None;
        };
        let (BindingPattern::BindingIdentifier(param1), BindingPattern::BindingIdentifier(param2)) =
            (&first.pattern, &second.pattern)
        else {
            return None;
        };
        if first.initializer.is_some() || second.initializer.is_some() || param1.name == param2.name {
            return None;
        }
        let (Expression::Identifier(left), Expression::Identifier(right)) = (&binary.left, &binary.right) else {
            return None;
        };
        (left.name == param1.name && right.name == param2.name).then_some(OperatorProxyInfo {
            operator: binary.operator,
        })
    }

    fn add(&mut self, symbol: Option<SymbolId>, func: &Function<'_>) {
        if let Some(symbol) = symbol
            && self.binary_calls_only.contains(&symbol)
            && let Some(info) = Self::extract_proxy_info(func)
        {
            deob_trace!("[AST] Found operator proxy -> {:?}", info.operator);
            self.proxies.insert(symbol, info);
        }
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for OperatorProxyCollector {
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
