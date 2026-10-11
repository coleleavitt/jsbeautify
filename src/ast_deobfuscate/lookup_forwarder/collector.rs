use oxc_ast::ast::{BindingPattern, CallExpression, Expression, Function, Program, Statement, VariableDeclarator};
use oxc_semantic::Scoping;
use oxc_syntax::scope::ScopeId;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::Traverse;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::call_sites::plain_call_arities;
use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;
use super::types::LookupForwarderInfo;

/// Finds `function F(x) { return A()[x]; }` bindings whose every reference is
/// a one-argument call at which `A` names the same binding.
#[derive(Default)]
pub struct LookupForwarderCollector {
    unary_calls_only: FxHashSet<SymbolId>,
    candidates: FxHashMap<SymbolId, LookupForwarderInfo>,
    call_scopes: FxHashMap<SymbolId, Vec<ScopeId>>,
    forwarders: FxHashMap<SymbolId, LookupForwarderInfo>,
}

impl LookupForwarderCollector {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn forwarders(self) -> FxHashMap<SymbolId, LookupForwarderInfo> {
        self.forwarders
    }

    fn extract_info(func: &Function<'_>, scoping: &Scoping) -> Option<LookupForwarderInfo> {
        if func.r#async
            || func.generator
            || func.params.rest.is_some()
            || func.params.items.iter().any(|param| param.initializer.is_some())
        {
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
            accessor_symbol: scoping.get_reference(accessor.reference_id()).symbol_id(),
        })
    }

    fn add(&mut self, symbol: Option<SymbolId>, func: &Function<'_>, scoping: &Scoping) {
        if let Some(symbol) = symbol
            && self.unary_calls_only.contains(&symbol)
            && let Some(info) = Self::extract_info(func, scoping)
        {
            self.candidates.insert(symbol, info);
        }
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for LookupForwarderCollector {
    fn enter_program(&mut self, program: &mut Program<'a>, ctx: &mut Ctx<'a>) {
        self.unary_calls_only = plain_call_arities(program, ctx.scoping())
            .into_iter()
            .filter(|(_, arities)| arities.len() == 1 && arities.contains(&1))
            .map(|(symbol, _)| symbol)
            .collect();
    }

    fn enter_function(&mut self, func: &mut Function<'a>, ctx: &mut Ctx<'a>) {
        if func.is_declaration() {
            self.add(func.id.as_ref().and_then(|id| id.symbol_id.get()), func, ctx.scoping());
        }
    }

    fn enter_variable_declarator(&mut self, declarator: &mut VariableDeclarator<'a>, ctx: &mut Ctx<'a>) {
        if let BindingPattern::BindingIdentifier(id) = &declarator.id
            && let Some(Expression::FunctionExpression(func)) = &declarator.init
        {
            self.add(id.symbol_id.get(), func, ctx.scoping());
        }
    }

    fn enter_call_expression(&mut self, call: &mut CallExpression<'a>, ctx: &mut Ctx<'a>) {
        if let Expression::Identifier(ident) = &call.callee
            && let Some(symbol) = ctx.scoping().get_reference(ident.reference_id()).symbol_id()
        {
            self.call_scopes.entry(symbol).or_default().push(ctx.current_scope_id());
        }
    }

    fn exit_program(&mut self, _program: &mut Program<'a>, ctx: &mut Ctx<'a>) {
        let scoping = ctx.scoping();
        for (symbol, info) in std::mem::take(&mut self.candidates) {
            let accessor_resolves = self.call_scopes.get(&symbol).is_some_and(|scopes| {
                scopes.iter().all(|&scope| {
                    scoping.find_binding(scope, info.accessor_name.as_str().into()) == info.accessor_symbol
                })
            });
            if accessor_resolves {
                eprintln!("[AST/lookup-fwd] found forwarder to {}()[x]", info.accessor_name);
                self.forwarders.insert(symbol, info);
            }
        }
    }
}
