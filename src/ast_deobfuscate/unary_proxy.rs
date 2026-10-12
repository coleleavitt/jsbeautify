//! Unary operator proxy inlining.
//!
//! Detects and inlines `function f(x){return !x}`, `function f(x){return -x}`,
//! `function f(x){return +x}`, `function f(x){return ~x}`, `function f(x){return typeof x}`,
//! `function f(x){return void x}` — whether declared as `function f(x){...}` or
//! `var f = function(x){...};`. Every call site `f(arg)` becomes the corresponding
//! `<op>arg` expression and the declaration is removed. Proxies are matched by
//! `SymbolId` and only when every reference is a one-argument call.
//!
//! Akamai BMP defines many of these (`Bd(x) = !x`, `lv(x) = -x`, `Xv(x) = +x`) and
//! calls them hundreds of times — the existing [`OperatorProxyCollector`] only
//! handles 2-argument binary proxies, so this complements it.

use oxc_allocator::{CloneIn, GetAllocator};
use oxc_ast::ast::{
    BindingPattern, CallExpression, Expression, Function, Program, Statement, UnaryOperator, VariableDeclarator,
};
use oxc_span::SPAN;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::call_sites::plain_call_arities;
use crate::ast_deobfuscate::rename_guards::is_annex_b_function;
use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

#[derive(Debug, Clone, Copy)]
pub struct UnaryProxyInfo {
    pub operator: UnaryOperator,
}

#[derive(Default)]
pub struct UnaryProxyCollector {
    unary_calls_only: FxHashSet<SymbolId>,
    proxies: FxHashMap<SymbolId, UnaryProxyInfo>,
}

impl UnaryProxyCollector {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn get_proxies(&self) -> FxHashMap<SymbolId, UnaryProxyInfo> {
        self.proxies.clone()
    }

    fn extract_proxy_info(func: &Function<'_>) -> Option<UnaryProxyInfo> {
        if func.r#async || func.generator {
            return None;
        }
        let body = func.body.as_ref()?;
        let [Statement::ReturnStatement(ret)] = body.statements.as_slice() else {
            return None;
        };
        let Some(Expression::UnaryExpression(unary)) = &ret.argument else {
            return None;
        };
        // `delete param` is a no-op on a binding but deletes a property inline.
        if unary.operator == UnaryOperator::Delete || func.params.items.len() != 1 || func.params.rest.is_some() {
            return None;
        }
        let param = &func.params.items[0];
        let BindingPattern::BindingIdentifier(param_id) = &param.pattern else {
            return None;
        };
        let Expression::Identifier(arg_ident) = &unary.argument else {
            return None;
        };
        if param.initializer.is_some() || arg_ident.name != param_id.name {
            return None;
        }
        Some(UnaryProxyInfo {
            operator: unary.operator,
        })
    }

    fn add(&mut self, symbol: Option<SymbolId>, func: &Function<'_>) {
        if let Some(symbol) = symbol
            && self.unary_calls_only.contains(&symbol)
            && let Some(info) = Self::extract_proxy_info(func)
        {
            deob_trace!("[AST] Found unary proxy -> {:?}", info.operator);
            self.proxies.insert(symbol, info);
        }
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for UnaryProxyCollector {
    fn enter_program(&mut self, program: &mut Program<'a>, ctx: &mut Ctx<'a>) {
        self.unary_calls_only = plain_call_arities(program, ctx.scoping())
            .into_iter()
            .filter(|(_, arities)| arities.len() == 1 && arities.contains(&1))
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

pub struct UnaryProxyInliner {
    proxies: FxHashMap<SymbolId, UnaryProxyInfo>,
    inlined: usize,
}

impl UnaryProxyInliner {
    #[must_use]
    pub const fn new(proxies: FxHashMap<SymbolId, UnaryProxyInfo>) -> Self {
        Self { proxies, inlined: 0 }
    }

    #[must_use]
    pub const fn inlined_count(&self) -> usize {
        self.inlined
    }

    fn is_proxy(&self, symbol: Option<SymbolId>) -> bool {
        symbol.is_some_and(|symbol| self.proxies.contains_key(&symbol))
    }

    fn try_inline_call<'a>(&mut self, call: &CallExpression<'a>, ctx: &mut Ctx<'a>) -> Option<Expression<'a>> {
        let Expression::Identifier(ident) = &call.callee else {
            return None;
        };
        let symbol = ctx.scoping().get_reference(ident.reference_id()).symbol_id()?;
        let info = self.proxies.get(&symbol)?;
        let arg = call.arguments.first()?.as_expression()?;
        if self.inlined < 10 {
            deob_trace!("[AST] Inlining unary proxy: {} -> {:?}", ident.name, info.operator);
        }
        self.inlined += 1;
        Some(Expression::new_unary_expression(
            SPAN,
            info.operator,
            arg.clone_in_with_semantic_ids(ctx.allocator()),
            ctx,
        ))
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for UnaryProxyInliner {
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        if let Expression::CallExpression(call) = expr
            && let Some(inlined) = self.try_inline_call(call, ctx)
        {
            *expr = inlined;
        }
    }

    fn exit_statement(&mut self, stmt: &mut Statement<'a>, ctx: &mut Ctx<'a>) {
        let remove = match stmt {
            Statement::FunctionDeclaration(func) => {
                !is_annex_b_function(ctx, func) && self.is_proxy(func.id.as_ref().and_then(|id| id.symbol_id.get()))
            }
            Statement::VariableDeclaration(decl) => decl.declarations.iter().all(|d| {
                let BindingPattern::BindingIdentifier(id) = &d.id else {
                    return false;
                };
                matches!(&d.init, Some(Expression::FunctionExpression(_))) && self.is_proxy(id.symbol_id.get())
            }),
            _ => false,
        };
        if remove {
            *stmt = Statement::new_empty_statement(SPAN, ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast_deobfuscate::semantic;
    use oxc_allocator::Allocator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_span::SourceType;
    use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

    fn run(code: &str) -> String {
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let mut program = ret.program;

        let mut collector = UnaryProxyCollector::new();
        let state = DeobfuscateState::new();
        let scoping = semantic::scoping(&program);
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);
        traverse_mut_with_ctx(&mut collector, &mut program, &mut ctx);
        let proxies = collector.get_proxies();

        if !proxies.is_empty() {
            let mut inliner = UnaryProxyInliner::new(proxies);
            let scoping = semantic::scoping(&program);
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
            traverse_mut_with_ctx(&mut inliner, &mut program, &mut ctx);
        }

        Codegen::new().build(&program).code
    }

    #[test]
    fn var_negation_proxy() {
        let out = run("var Bd = function(x) { return !x; }; var r = Bd(y);");
        assert!(out.contains("!y"), "got: {out}");
        assert!(!out.contains("Bd"), "proxy should be removed: {out}");
    }

    #[test]
    fn function_decl_negation_proxy() {
        let out = run("function neg(x) { return -x; } var r = neg(z);");
        assert!(out.contains("-z"), "got: {out}");
        assert!(!out.contains("function neg"), "proxy should be removed: {out}");
    }

    #[test]
    fn bitwise_not() {
        let out = run("var bn = function(x) { return ~x; }; var r = bn(y);");
        assert!(out.contains("~y"), "got: {out}");
    }

    #[test]
    fn typeof_proxy() {
        let out = run("var tp = function(x) { return typeof x; }; var r = tp(y);");
        assert!(out.contains("typeof y"), "got: {out}");
    }

    #[test]
    fn leaves_non_proxies_alone() {
        let out = run("function f(x) { return x + 1; } var r = f(y);");
        assert!(out.contains("function f"), "non-proxy must be kept: {out}");
        assert!(out.contains("f(y)"), "non-proxy call must be kept: {out}");
    }

    #[test]
    fn delete_proxy_is_not_inlined() {
        let out = run("function d(x) { return delete x; }\nvar o = { p: 1 };\nd(o.p);");
        assert!(out.contains("d(o.p)"), "got: {out}");
    }

    #[test]
    fn proxy_used_as_a_value_is_kept() {
        let out = run("function not(x) { return !x; }\nvar r = [0, 1].filter(not);\nvar s = not(1);");
        assert!(out.contains("function not"), "got: {out}");
        assert!(out.contains("not(1)"), "got: {out}");
        let out = run("function neg(x) { return -x; }\nneg(1);\nneg(1, 2);");
        assert!(out.contains("function neg"), "other arities keep the proxy: {out}");
    }

    #[test]
    fn same_name_in_another_scope_is_untouched() {
        let out =
            run("function f() { function neg(x) { return -x; } return neg(1); }\nfunction g(neg) { return neg(2); }");
        assert!(out.contains("return -1"), "got: {out}");
        assert!(out.contains("return neg(2)"), "got: {out}");
    }
}
