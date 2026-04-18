//! Unary operator proxy inlining.
//!
//! Detects and inlines `function f(x){return !x}`, `function f(x){return -x}`,
//! `function f(x){return +x}`, `function f(x){return ~x}`, `function f(x){return typeof x}`,
//! `function f(x){return void x}` — whether declared as `function f(x){...}` or
//! `var f = function(x){...};`. Every call site `f(arg)` becomes the corresponding
//! `<op>arg` expression and the declaration is removed.
//!
//! Akamai BMP defines many of these (`Bd(x) = !x`, `lv(x) = -x`, `Xv(x) = +x`) and
//! calls them hundreds of times — the existing [`OperatorProxyCollector`] only
//! handles 2-argument binary proxies, so this complements it.

use oxc_allocator::CloneIn;
use oxc_ast::ast::{
    BindingPattern, CallExpression, EmptyStatement, Expression, Function, Statement, UnaryExpression, UnaryOperator,
    VariableDeclarator,
};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::FxHashMap;
use std::cell::Cell;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

#[derive(Debug, Clone, Copy)]
pub struct UnaryProxyInfo {
    pub operator: UnaryOperator,
}

pub struct UnaryProxyCollector {
    proxies: FxHashMap<String, UnaryProxyInfo>,
}

impl UnaryProxyCollector {
    #[must_use]
    pub fn new() -> Self {
        Self {
            proxies: FxHashMap::default(),
        }
    }

    #[must_use]
    pub fn get_proxies(&self) -> FxHashMap<String, UnaryProxyInfo> {
        self.proxies.clone()
    }

    fn extract_proxy_info(func: &Function<'_>, name: &str) -> Option<UnaryProxyInfo> {
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
        let Some(Expression::UnaryExpression(unary)) = &ret.argument else {
            return None;
        };
        if func.params.items.len() != 1 {
            return None;
        }
        let BindingPattern::BindingIdentifier(param) = &func.params.items[0].pattern else {
            return None;
        };
        let Expression::Identifier(arg_ident) = &unary.argument else {
            return None;
        };
        if arg_ident.name.as_str() != param.name.as_str() {
            return None;
        }
        eprintln!("[AST] Found unary proxy: {name} -> {:?}", unary.operator);
        Some(UnaryProxyInfo {
            operator: unary.operator,
        })
    }
}

impl Default for UnaryProxyCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for UnaryProxyCollector {
    fn enter_function(&mut self, func: &mut Function<'a>, _ctx: &mut Ctx<'a>) {
        if let Some(id) = &func.id {
            let name = id.name.as_str().to_string();
            if let Some(info) = Self::extract_proxy_info(func, &name) {
                self.proxies.insert(name, info);
            }
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
        if let Some(info) = Self::extract_proxy_info(func, &name) {
            self.proxies.insert(name, info);
        }
    }
}

pub struct UnaryProxyInliner {
    proxies: FxHashMap<String, UnaryProxyInfo>,
    inlined: usize,
}

impl UnaryProxyInliner {
    #[must_use]
    pub fn new(proxies: FxHashMap<String, UnaryProxyInfo>) -> Self {
        Self { proxies, inlined: 0 }
    }

    #[must_use]
    pub const fn inlined_count(&self) -> usize {
        self.inlined
    }

    fn try_inline_call<'a>(&mut self, call: &CallExpression<'a>, ctx: &mut Ctx<'a>) -> Option<Expression<'a>> {
        let Expression::Identifier(ident) = &call.callee else {
            return None;
        };
        let name = ident.name.as_str();
        let info = self.proxies.get(name)?;
        if call.arguments.len() != 1 {
            return None;
        }
        let arg = call.arguments[0].as_expression()?;
        if self.inlined < 10 {
            eprintln!("[AST] Inlining unary proxy: {name} -> {:?}", info.operator);
        }
        self.inlined += 1;
        Some(Expression::UnaryExpression(ctx.ast.alloc(UnaryExpression {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            operator: info.operator,
            argument: arg.clone_in_with_semantic_ids(ctx.ast.allocator),
        })))
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
        if let Statement::FunctionDeclaration(func) = stmt
            && let Some(id) = &func.id
            && self.proxies.contains_key(id.name.as_str())
        {
            eprintln!("[AST] Removing unary proxy function: {}", id.name.as_str());
            *stmt = Statement::EmptyStatement(ctx.ast.alloc(EmptyStatement {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
            }));
            return;
        }
        if let Statement::VariableDeclaration(decl) = stmt {
            let all_are_proxies = !decl.declarations.is_empty()
                && decl.declarations.iter().all(|d| {
                    let BindingPattern::BindingIdentifier(id) = &d.id else {
                        return false;
                    };
                    let Some(Expression::FunctionExpression(_)) = &d.init else {
                        return false;
                    };
                    self.proxies.contains_key(id.name.as_str())
                });
            if all_are_proxies {
                let names: Vec<String> = decl
                    .declarations
                    .iter()
                    .filter_map(|d| match &d.id {
                        BindingPattern::BindingIdentifier(id) => Some(id.name.as_str().to_string()),
                        _ => None,
                    })
                    .collect();
                eprintln!("[AST] Removing unary proxy var declaration: var {};", names.join(", "));
                *stmt = Statement::EmptyStatement(ctx.ast.alloc(EmptyStatement {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                }));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_allocator::Allocator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_semantic::SemanticBuilder;
    use oxc_span::SourceType;
    use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

    fn run(code: &str) -> String {
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let mut program = ret.program;

        let mut collector = UnaryProxyCollector::new();
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);
        traverse_mut_with_ctx(&mut collector, &mut program, &mut ctx);
        let proxies = collector.get_proxies();

        if !proxies.is_empty() {
            let mut inliner = UnaryProxyInliner::new(proxies);
            let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
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
}
