use oxc_allocator::CloneIn;
use oxc_ast::ast::{
    BinaryExpression, BinaryOperator, BindingPattern, CallExpression, EmptyStatement, Expression, Function, Statement,
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
pub struct OperatorProxyInfo {
    pub operator: BinaryOperator,
    pub param1: String,
    pub param2: String,
}

pub struct OperatorProxyCollector {
    proxies: FxHashMap<String, OperatorProxyInfo>,
}

impl OperatorProxyCollector {
    #[must_use]
    pub fn new() -> Self {
        Self {
            proxies: FxHashMap::default(),
        }
    }

    #[must_use]
    pub fn get_proxies(&self) -> FxHashMap<String, OperatorProxyInfo> {
        self.proxies.clone()
    }

    fn try_extract_proxy(&mut self, func: &Function<'_>) -> Option<(String, OperatorProxyInfo)> {
        let name = func.id.as_ref()?.name.as_str().to_string();
        let info = Self::extract_proxy_info(func, &name)?;
        Some((name, info))
    }

    fn try_extract_proxy_for(&mut self, name: &str, func: &Function<'_>) -> Option<OperatorProxyInfo> {
        Self::extract_proxy_info(func, name)
    }

    fn extract_proxy_info(func: &Function<'_>, name: &str) -> Option<OperatorProxyInfo> {
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

        let Some(Expression::BinaryExpression(binary)) = &ret.argument else {
            return None;
        };

        if func.params.items.len() != 2 {
            return None;
        }

        let param1 = if let BindingPattern::BindingIdentifier(ident) = &func.params.items[0].pattern {
            ident.name.as_str().to_string()
        } else {
            return None;
        };

        let param2 = if let BindingPattern::BindingIdentifier(ident) = &func.params.items[1].pattern {
            ident.name.as_str().to_string()
        } else {
            return None;
        };

        let Expression::Identifier(left_ident) = &binary.left else {
            return None;
        };
        let Expression::Identifier(right_ident) = &binary.right else {
            return None;
        };

        if left_ident.name.as_str() != param1 || right_ident.name.as_str() != param2 {
            return None;
        }

        eprintln!("[AST] Found operator proxy: {name} -> {:?}", binary.operator);

        Some(OperatorProxyInfo {
            operator: binary.operator,
            param1,
            param2,
        })
    }
}

impl Default for OperatorProxyCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for OperatorProxyCollector {
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
}

pub struct OperatorProxyInliner {
    proxies: FxHashMap<String, OperatorProxyInfo>,
    changed: bool,
}

impl OperatorProxyInliner {
    #[must_use]
    pub const fn new(proxies: FxHashMap<String, OperatorProxyInfo>) -> Self {
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

        if call.arguments.len() != 2 {
            return None;
        }

        let arg1 = call.arguments[0].as_expression()?;
        let arg2 = call.arguments[1].as_expression()?;

        eprintln!("[AST] Inlining operator proxy: {} -> {:?}", name, proxy.operator);
        self.changed = true;

        Some(Expression::BinaryExpression(ctx.ast.alloc(BinaryExpression {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            left: Self::clone_expression(arg1, ctx),
            operator: proxy.operator,
            right: Self::clone_expression(arg2, ctx),
        })))
    }

    fn clone_expression<'a>(expr: &Expression<'a>, ctx: &mut Ctx<'a>) -> Expression<'a> {
        expr.clone_in_with_semantic_ids(ctx.ast.allocator)
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for OperatorProxyInliner {
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
                eprintln!("[AST] Removing operator proxy function: {name}");
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
                eprintln!(
                    "[AST] Removing operator proxy var declaration: var {};",
                    names.join(", ")
                );
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

    fn run_operator_proxy(code: &str) -> String {
        let allocator = Allocator::default();
        let source_type = SourceType::mjs();
        let ret = Parser::new(&allocator, code, source_type).parse();
        let mut program = ret.program;

        let mut collector = OperatorProxyCollector::new();
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

        traverse_mut_with_ctx(&mut collector, &mut program, &mut ctx);

        let proxies = collector.get_proxies();
        eprintln!("Operator proxies: {:?}", proxies.keys().collect::<Vec<_>>());

        if !proxies.is_empty() {
            let mut inliner = OperatorProxyInliner::new(proxies);
            let state = DeobfuscateState::new();
            let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

            traverse_mut_with_ctx(&mut inliner, &mut program, &mut ctx);
        }

        Codegen::new().build(&program).code
    }

    #[test]
    fn test_detect_add_proxy() {
        let output = run_operator_proxy("function _0xadd(a, b) { return a + b; } var x = _0xadd(5, 10);");
        eprintln!("Output: {output}");
        assert!(
            !output.contains("function _0xadd"),
            "Proxy function should be removed, got: {output}"
        );
        assert!(
            output.contains("5 + 10") || output.contains("5+10"),
            "Call should be inlined to binary op, got: {output}"
        );
    }

    #[test]
    fn test_detect_multiply_proxy() {
        let output = run_operator_proxy("function _mul(a, b) { return a * b; } var x = _mul(3, 4);");
        eprintln!("Output: {output}");
        assert!(
            !output.contains("function _mul"),
            "Proxy function should be removed, got: {output}"
        );
        assert!(
            output.contains("3 * 4") || output.contains("3*4"),
            "Call should be inlined to binary op, got: {output}"
        );
    }

    #[test]
    fn test_multiple_operators() {
        let output = run_operator_proxy(
            "function _add(a, b) { return a + b; } function _mul(a, b) { return a * b; } var x = _add(1, 2); var y = _mul(3, 4);",
        );
        eprintln!("Output: {output}");
        assert!(
            output.contains("1 + 2") || output.contains("1+2"),
            "Add should be inlined, got: {output}"
        );
        assert!(
            output.contains("3 * 4") || output.contains("3*4"),
            "Mul should be inlined, got: {output}"
        );
    }

    #[test]
    fn test_comparison_operator() {
        let output = run_operator_proxy("function _lt(a, b) { return a < b; } var x = _lt(5, 10);");
        eprintln!("Output: {output}");
        assert!(
            output.contains("5 < 10") || output.contains("5<10"),
            "Call should be inlined to comparison, got: {output}"
        );
    }
}
