use oxc_allocator::CloneIn;
use oxc_ast::ast::{
    Argument, BindingPattern, CallExpression, EmptyStatement, Expression, Function, IdentifierReference, Statement,
    VariableDeclarator,
};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::FxHashMap;
use std::cell::Cell;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

#[derive(Debug, Clone)]
pub struct CallProxyInfo {
    pub target_name: String,
    pub params: Vec<String>,
}

pub struct CallProxyCollector {
    proxies: FxHashMap<String, CallProxyInfo>,
    call_counts: FxHashMap<String, usize>,
}

impl CallProxyCollector {
    #[must_use]
    pub fn new() -> Self {
        Self {
            proxies: FxHashMap::default(),
            call_counts: FxHashMap::default(),
        }
    }

    #[must_use]
    pub fn get_single_use_proxies(&self) -> FxHashMap<String, CallProxyInfo> {
        self.proxies
            .iter()
            .filter(|(name, _)| {
                let count = self.call_counts.get(*name).copied().unwrap_or(0);
                count == 1
            })
            .map(|(name, info)| (name.clone(), info.clone()))
            .collect()
    }

    fn try_extract_proxy(&mut self, func: &Function<'_>) -> Option<(String, CallProxyInfo)> {
        let name = func.id.as_ref()?.name.as_str().to_string();
        let info = Self::extract_proxy_info(func, &name)?;
        Some((name, info))
    }

    fn try_extract_proxy_for(&mut self, name: &str, func: &Function<'_>) -> Option<CallProxyInfo> {
        Self::extract_proxy_info(func, name)
    }

    fn extract_proxy_info(func: &Function<'_>, name: &str) -> Option<CallProxyInfo> {
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

        let Expression::Identifier(target_ident) = &call.callee else {
            return None;
        };

        let target_name = target_ident.name.as_str().to_string();

        let params: Vec<String> = func
            .params
            .items
            .iter()
            .filter_map(|p| {
                if let BindingPattern::BindingIdentifier(ident) = &p.pattern {
                    Some(ident.name.as_str().to_string())
                } else {
                    None
                }
            })
            .collect();

        if params.len() != func.params.items.len() {
            return None;
        }

        if call.arguments.len() != params.len() {
            return None;
        }

        for (i, arg) in call.arguments.iter().enumerate() {
            let arg_expr = arg.as_expression()?;
            let Expression::Identifier(arg_ident) = arg_expr else {
                return None;
            };
            if arg_ident.name.as_str() != params[i] {
                return None;
            }
        }

        eprintln!(
            "[AST] Found call proxy: {} -> {} ({} params)",
            name,
            target_name,
            params.len()
        );

        Some(CallProxyInfo { target_name, params })
    }
}

impl Default for CallProxyCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for CallProxyCollector {
    fn enter_function(&mut self, func: &mut Function<'a>, _ctx: &mut Ctx<'a>) {
        if let Some((name, info)) = self.try_extract_proxy(func) {
            self.proxies.insert(name, info);
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
        if let Some(info) = self.try_extract_proxy_for(&name, func) {
            self.proxies.insert(name, info);
        }
    }

    fn enter_call_expression(&mut self, call: &mut CallExpression<'a>, _ctx: &mut Ctx<'a>) {
        if let Expression::Identifier(ident) = &call.callee {
            let name = ident.name.as_str().to_string();
            *self.call_counts.entry(name).or_insert(0) += 1;
        }
    }
}

pub struct CallProxyInliner {
    proxies: FxHashMap<String, CallProxyInfo>,
    changed: bool,
}

impl CallProxyInliner {
    #[must_use]
    pub const fn new(proxies: FxHashMap<String, CallProxyInfo>) -> Self {
        Self {
            proxies,
            changed: false,
        }
    }

    #[must_use]
    pub const fn has_changed(&self) -> bool {
        self.changed
    }

    fn try_inline_call<'a>(&mut self, call: &CallExpression<'a>, ctx: &mut Ctx<'a>) -> Option<Expression<'a>> {
        let name = if let Expression::Identifier(ident) = &call.callee {
            ident.name.as_str()
        } else {
            return None;
        };

        let proxy = self.proxies.get(name)?;

        if call.arguments.len() != proxy.params.len() {
            return None;
        }

        eprintln!("[AST] Inlining call proxy: {} -> {}", name, proxy.target_name);
        self.changed = true;

        let mut arguments = ctx.ast.vec();
        for arg in &call.arguments {
            if let Some(expr) = arg.as_expression() {
                arguments.push(Argument::from(Self::clone_expression(expr, ctx)));
            }
        }

        Some(Expression::CallExpression(ctx.ast.alloc(CallExpression {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            callee: Expression::Identifier(ctx.ast.alloc(IdentifierReference {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                name: ctx.ast.ident(&proxy.target_name),
                reference_id: Cell::default(),
            })),
            arguments,
            optional: false,
            type_arguments: None,
            pure: false,
        })))
    }

    fn clone_expression<'a>(expr: &Expression<'a>, ctx: &mut Ctx<'a>) -> Expression<'a> {
        expr.clone_in_with_semantic_ids(ctx.ast.allocator)
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for CallProxyInliner {
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
        {
            let name = id.name.as_str();
            if self.proxies.contains_key(name) {
                eprintln!("[AST] Removing call proxy function: {name}");
                self.changed = true;
                *stmt = Statement::EmptyStatement(ctx.ast.alloc(EmptyStatement {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                }));
                return;
            }
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
                eprintln!("[AST] Removing call proxy var declaration: var {};", names.join(", "));
                self.changed = true;
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
    use crate::ast_deobfuscate::DeobfuscateState;
    use oxc_allocator::Allocator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_semantic::SemanticBuilder;
    use oxc_span::SourceType;
    use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

    fn run_call_proxy(code: &str) -> String {
        let allocator = Allocator::default();
        let source_type = SourceType::mjs();
        let ret = Parser::new(&allocator, code, source_type).parse();
        let mut program = ret.program;

        let mut collector = CallProxyCollector::new();
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

        traverse_mut_with_ctx(&mut collector, &mut program, &mut ctx);

        let single_use = collector.get_single_use_proxies();
        eprintln!("Single use proxies: {:?}", single_use.keys().collect::<Vec<_>>());

        if !single_use.is_empty() {
            let mut inliner = CallProxyInliner::new(single_use);
            let state = DeobfuscateState::new();
            let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

            traverse_mut_with_ctx(&mut inliner, &mut program, &mut ctx);
        }

        Codegen::new().build(&program).code
    }

    #[test]
    fn test_detect_simple_proxy() {
        let output = run_call_proxy("function _0xabc(p) { return _0xdec(p); } var x = _0xabc(123);");
        eprintln!("Output: {output}");
        assert!(
            !output.contains("function _0xabc"),
            "Proxy function should be removed, got: {output}"
        );
        assert!(
            output.contains("_0xdec(123)"),
            "Call should be inlined to target, got: {output}"
        );
    }

    #[test]
    fn test_multi_param_proxy() {
        let output =
            run_call_proxy("function _wrap(a, b, c) { return _target(a, b, c); } var result = _wrap(1, 2, 3);");
        eprintln!("Output: {output}");
        assert!(
            !output.contains("function _wrap"),
            "Proxy function should be removed, got: {output}"
        );
        assert!(
            output.contains("_target(1, 2, 3)") || output.contains("_target(1,2,3)"),
            "Call should be inlined to target, got: {output}"
        );
    }

    #[test]
    fn test_preserve_multi_use_proxy() {
        let output = run_call_proxy("function _wrap(x) { return _target(x); } var a = _wrap(1); var b = _wrap(2);");
        eprintln!("Output: {output}");
        assert!(
            output.contains("function _wrap"),
            "Multi-use proxy should be preserved, got: {output}"
        );
    }

    #[test]
    fn test_no_proxy_wrong_order() {
        let output = run_call_proxy("function _wrap(a, b) { return _target(b, a); } var x = _wrap(1, 2);");
        eprintln!("Output: {output}");
        assert!(
            output.contains("function _wrap"),
            "Non-proxy (wrong param order) should be preserved, got: {output}"
        );
    }
}
