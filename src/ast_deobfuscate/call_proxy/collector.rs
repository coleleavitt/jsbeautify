use oxc_ast::ast::{
    Argument, BindingPattern, CallExpression, Expression, Function, Program, Statement, VariableDeclarator,
};
use oxc_semantic::Scoping;
use oxc_span::SourceType;
use oxc_syntax::scope::ScopeId;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::Traverse;
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::rename_guards::is_global_binding;
use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;
use super::types::CallProxyInfo;

/// A call through a binding's identifier.
struct CallSite {
    scope_id: ScopeId,
    arg_count: usize,
    has_spread: bool,
}

/// Finds `function p(a, b) { return t(a, b); }` proxies whose only reference
/// is one call `p(x, y)` that can become `t(x, y)`: same arity, and `t`
/// resolving to the same binding at the call site.
#[derive(Default)]
pub struct CallProxyCollector {
    candidates: FxHashMap<SymbolId, CallProxyInfo>,
    call_sites: FxHashMap<SymbolId, Vec<CallSite>>,
    proxies: FxHashMap<SymbolId, CallProxyInfo>,
    source_type: SourceType,
}

impl CallProxyCollector {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn get_single_use_proxies(&self) -> FxHashMap<SymbolId, CallProxyInfo> {
        self.proxies.clone()
    }

    fn extract_proxy_info(func: &Function<'_>, scoping: &Scoping) -> Option<CallProxyInfo> {
        if func.r#async || func.generator || func.params.rest.is_some() {
            return None;
        }
        let params = func
            .params
            .items
            .iter()
            .map(|param| match &param.pattern {
                BindingPattern::BindingIdentifier(ident) if param.initializer.is_none() => ident.symbol_id.get(),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()?;

        let body = func.body.as_ref()?;
        let [Statement::ReturnStatement(ret)] = body.statements.as_slice() else {
            return None;
        };
        let Some(Expression::CallExpression(call)) = &ret.argument else {
            return None;
        };
        let Expression::Identifier(target) = &call.callee else {
            return None;
        };
        // A direct eval sees the proxy's own scope and context.
        if target.name == "eval" || call.arguments.len() != params.len() {
            return None;
        }
        let target_symbol = scoping.get_reference(target.reference_id()).symbol_id();
        if target_symbol.is_some_and(|symbol| params.contains(&symbol) || scoping.symbol_is_mutated(symbol)) {
            return None;
        }
        let forwards_params = call.arguments.iter().zip(&params).all(|(arg, &param)| {
            let Argument::Identifier(arg) = arg else {
                return false;
            };
            scoping.get_reference(arg.reference_id()).symbol_id() == Some(param)
        });
        forwards_params.then(|| CallProxyInfo {
            target_name: target.name.to_string(),
            target_symbol,
            params: params.len(),
        })
    }

    fn add_candidate(&mut self, symbol: Option<SymbolId>, func: &Function<'_>, scoping: &Scoping) {
        if let Some(symbol) = symbol
            && let Some(info) = Self::extract_proxy_info(func, scoping)
        {
            self.candidates.insert(symbol, info);
        }
    }

    fn is_single_use(&self, symbol: SymbolId, info: &CallProxyInfo, scoping: &Scoping) -> bool {
        let Some([call]) = self.call_sites.get(&symbol).map(Vec::as_slice) else {
            return false;
        };
        !scoping.symbol_is_mutated(symbol)
            && !is_global_binding(self.source_type, scoping, symbol)
            && scoping.symbol_redeclarations(symbol).is_empty()
            && scoping.get_resolved_reference_ids(symbol).len() == 1
            && !call.has_spread
            && call.arg_count == info.params
            && scoping.find_binding(call.scope_id, info.target_name.as_str().into()) == info.target_symbol
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for CallProxyCollector {
    fn enter_program(&mut self, program: &mut Program<'a>, _ctx: &mut Ctx<'a>) {
        self.source_type = program.source_type;
    }

    fn enter_function(&mut self, func: &mut Function<'a>, ctx: &mut Ctx<'a>) {
        if func.is_declaration() {
            let symbol = func.id.as_ref().and_then(|id| id.symbol_id.get());
            self.add_candidate(symbol, func, ctx.scoping());
        }
    }

    fn enter_variable_declarator(&mut self, declarator: &mut VariableDeclarator<'a>, ctx: &mut Ctx<'a>) {
        if let BindingPattern::BindingIdentifier(id) = &declarator.id
            && let Some(Expression::FunctionExpression(func)) = &declarator.init
        {
            self.add_candidate(id.symbol_id.get(), func, ctx.scoping());
        }
    }

    fn enter_call_expression(&mut self, call: &mut CallExpression<'a>, ctx: &mut Ctx<'a>) {
        if let Expression::Identifier(ident) = &call.callee
            && let Some(symbol) = ctx.scoping().get_reference(ident.reference_id()).symbol_id()
        {
            self.call_sites.entry(symbol).or_default().push(CallSite {
                scope_id: ctx.current_scope_id(),
                arg_count: call.arguments.len(),
                has_spread: call.arguments.iter().any(|a| matches!(a, Argument::SpreadElement(_))),
            });
        }
    }

    fn exit_program(&mut self, _program: &mut Program<'a>, ctx: &mut Ctx<'a>) {
        let candidates = std::mem::take(&mut self.candidates);
        for (symbol, info) in candidates {
            if self.is_single_use(symbol, &info, ctx.scoping()) {
                eprintln!(
                    "[AST] Found call proxy -> {} ({} params)",
                    info.target_name, info.params
                );
                self.proxies.insert(symbol, info);
            }
        }
    }
}
