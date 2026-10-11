use oxc_ast::ast::{
    AssignmentTarget, BindingPattern, CallExpression, Expression, Function, IdentifierReference, Program, Statement,
    VariableDeclarator,
};
use oxc_semantic::Scoping;
use oxc_span::SourceType;
use oxc_syntax::scope::ScopeId;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::Traverse;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::function_context::this_free_functions;
use crate::ast_deobfuscate::rename_guards::is_global_binding;
use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;
use super::types::TrampolineInfo;

/// A trampoline binding and how it is defined.
struct Candidate {
    info: TrampolineInfo,
    /// Defined by `NAME = function …`, which is that binding's only write.
    assigned: bool,
}

#[derive(Default)]
pub struct TrampolineCollector {
    candidates: FxHashMap<SymbolId, Option<Candidate>>,
    call_scopes: FxHashMap<SymbolId, Vec<ScopeId>>,
    this_free: FxHashSet<SymbolId>,
    source_type: SourceType,
    trampolines: FxHashMap<SymbolId, TrampolineInfo>,
}

impl TrampolineCollector {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn into_trampolines(self) -> FxHashMap<SymbolId, TrampolineInfo> {
        self.trampolines
    }

    fn extract_info(func: &Function<'_>, scoping: &Scoping) -> Option<TrampolineInfo> {
        if func.r#async || func.generator {
            return None;
        }
        let body = func.body.as_ref()?;
        if !body.directives.is_empty() || body.statements.len() != 1 {
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
        if second.name.as_str() != "arguments" || resolve(second, scoping).is_some() {
            return None;
        }
        Some(TrampolineInfo {
            dispatcher: dispatcher.name.as_str().to_string(),
            dispatcher_symbol: resolve(dispatcher, scoping),
            state: state.name.as_str().to_string(),
            state_symbol: resolve(state, scoping),
            strict: scoping.scope_flags(func.scope_id()).is_strict_mode(),
        })
    }

    /// A binding defined as a trampoline more than once is never inlined.
    fn add_candidate(&mut self, symbol: Option<SymbolId>, func: &Function<'_>, assigned: bool, scoping: &Scoping) {
        if let Some(symbol) = symbol
            && let Some(info) = Self::extract_info(func, scoping)
        {
            let candidate = (!self.candidates.contains_key(&symbol)).then_some(Candidate { info, assigned });
            self.candidates.insert(symbol, candidate);
        }
    }

    /// Every reference is a call that the inliner rewrites to
    /// `DISPATCH(STATE, [args])` with `DISPATCH` and `STATE` still naming the
    /// trampoline's bindings, and dropping the trampoline's `this` is
    /// unobservable.
    fn is_inlinable(&self, symbol: SymbolId, candidate: &Candidate, scoping: &Scoping) -> bool {
        let info = &candidate.info;
        let call_scopes = self.call_scopes.get(&symbol).map_or(&[][..], Vec::as_slice);
        let writes = usize::from(candidate.assigned);
        let write_count = scoping
            .get_resolved_references(symbol)
            .filter(|reference| reference.is_write())
            .count();
        write_count == writes
            && !is_global_binding(self.source_type, scoping, symbol)
            && scoping.symbol_redeclarations(symbol).is_empty()
            && scoping.get_resolved_reference_ids(symbol).len() == call_scopes.len() + writes
            && (info.strict || info.dispatcher_symbol.is_some_and(|d| self.this_free.contains(&d)))
            && call_scopes.iter().all(|&scope| {
                scoping.find_binding(scope, info.dispatcher.as_str().into()) == info.dispatcher_symbol
                    && scoping.find_binding(scope, info.state.as_str().into()) == info.state_symbol
            })
    }
}

fn resolve(ident: &IdentifierReference<'_>, scoping: &Scoping) -> Option<SymbolId> {
    scoping.get_reference(ident.reference_id()).symbol_id()
}

impl<'a> Traverse<'a, DeobfuscateState> for TrampolineCollector {
    fn enter_program(&mut self, program: &mut Program<'a>, ctx: &mut Ctx<'a>) {
        self.this_free = this_free_functions(program, ctx.scoping());
        self.source_type = program.source_type;
    }

    fn enter_function(&mut self, func: &mut Function<'a>, ctx: &mut Ctx<'a>) {
        if func.is_declaration() {
            let symbol = func.id.as_ref().and_then(|id| id.symbol_id.get());
            self.add_candidate(symbol, func, false, ctx.scoping());
        }
    }

    fn enter_variable_declarator(&mut self, declarator: &mut VariableDeclarator<'a>, ctx: &mut Ctx<'a>) {
        if let BindingPattern::BindingIdentifier(id) = &declarator.id
            && let Some(Expression::FunctionExpression(func)) = &declarator.init
        {
            self.add_candidate(id.symbol_id.get(), func, false, ctx.scoping());
        }
    }

    fn enter_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        if let Expression::AssignmentExpression(assign) = expr
            && assign.operator.is_assign()
            && let AssignmentTarget::AssignmentTargetIdentifier(target) = &assign.left
            && let Expression::FunctionExpression(func) = &assign.right
        {
            let symbol = resolve(target, ctx.scoping());
            self.add_candidate(symbol, func, true, ctx.scoping());
        }
    }

    fn enter_call_expression(&mut self, call: &mut CallExpression<'a>, ctx: &mut Ctx<'a>) {
        if let Expression::Identifier(ident) = &call.callee
            && let Some(symbol) = resolve(ident, ctx.scoping())
        {
            self.call_scopes.entry(symbol).or_default().push(ctx.current_scope_id());
        }
    }

    fn exit_program(&mut self, _program: &mut Program<'a>, ctx: &mut Ctx<'a>) {
        let candidates = std::mem::take(&mut self.candidates);
        for (symbol, candidate) in candidates {
            let Some(candidate) = candidate else {
                continue;
            };
            if self.is_inlinable(symbol, &candidate, ctx.scoping()) {
                let info = candidate.info;
                eprintln!(
                    "[AST/tramp] found  {}.apply(this, [{}, arguments]) trampoline",
                    info.dispatcher, info.state
                );
                self.trampolines.insert(symbol, info);
            }
        }
    }
}
