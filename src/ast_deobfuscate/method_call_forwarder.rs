//! 2-arg method-call forwarder inliner.
//!
//! Detects wrappers that forward their two parameters to a fixed method call
//! on the first argument:
//!
//! ```text
//! function J(a, b) { return a.charCodeAt(b); }
//! var J = function(a, b) { return a.charCodeAt(b); };
//! ```
//!
//! Every call site `J(x, y)` becomes `x.charCodeAt(y)` and the wrapper
//! declaration is removed. The pass only fires when:
//! - exactly 2 params
//! - body is a single `return <param0>.<METHOD>(<param1>);`
//! - method name is a plain identifier (no computed member)
//! - no spread in either argument list
//!
//! This complements [`LookupForwarderInliner`] (which handles
//! `function F(x) { return acc()[x]; }`) by covering the other common
//! BMP/Jscrambler wrapper shape.

use oxc_allocator::CloneIn;
use oxc_ast::ast::{
    BindingPattern, CallExpression, EmptyStatement, Expression, Function, IdentifierName, Statement,
    StaticMemberExpression, VariableDeclarator,
};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::FxHashMap;
use std::cell::Cell;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

#[derive(Debug, Clone)]
pub struct MethodForwarderInfo {
    pub method: String,
}

pub struct MethodCallForwarderCollector {
    forwarders: FxHashMap<String, MethodForwarderInfo>,
}

impl MethodCallForwarderCollector {
    #[must_use]
    pub fn new() -> Self {
        Self {
            forwarders: FxHashMap::default(),
        }
    }

    #[must_use]
    pub fn into_forwarders(self) -> FxHashMap<String, MethodForwarderInfo> {
        self.forwarders
    }

    fn extract_info(func: &Function<'_>) -> Option<MethodForwarderInfo> {
        if func.r#async || func.generator {
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
}

impl Default for MethodCallForwarderCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for MethodCallForwarderCollector {
    fn enter_function(&mut self, func: &mut Function<'a>, _ctx: &mut Ctx<'a>) {
        if let Some(id) = &func.id
            && let Some(info) = Self::extract_info(func)
        {
            eprintln!(
                "[AST/method-fwd] found  function {}(a, b) {{ return a.{}(b); }}",
                id.name.as_str(),
                info.method
            );
            self.forwarders.insert(id.name.as_str().to_string(), info);
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
                "[AST/method-fwd] found  var {} = function(a, b) {{ return a.{}(b); }};",
                id.name.as_str(),
                info.method
            );
            self.forwarders.insert(id.name.as_str().to_string(), info);
        }
    }
}

pub struct MethodCallForwarderInliner {
    forwarders: FxHashMap<String, MethodForwarderInfo>,
    inlined: usize,
}

impl MethodCallForwarderInliner {
    #[must_use]
    pub fn new(forwarders: FxHashMap<String, MethodForwarderInfo>) -> Self {
        Self { forwarders, inlined: 0 }
    }

    #[must_use]
    pub const fn inlined(&self) -> usize {
        self.inlined
    }

    fn try_inline<'a>(&mut self, call: &CallExpression<'a>, ctx: &mut Ctx<'a>) -> Option<Expression<'a>> {
        let Expression::Identifier(ident) = &call.callee else {
            return None;
        };
        let info = self.forwarders.get(ident.name.as_str())?;
        if call.arguments.len() != 2 {
            return None;
        }
        let recv = call.arguments[0].as_expression()?;
        let arg = call.arguments[1].as_expression()?;
        if self.inlined < 10 {
            eprintln!(
                "[AST/method-fwd] inline {}(a, b) -> a.{}(b)",
                ident.name.as_str(),
                info.method
            );
        }
        self.inlined += 1;

        let recv_cloned = recv.clone_in_with_semantic_ids(ctx.ast.allocator);
        let arg_cloned = arg.clone_in_with_semantic_ids(ctx.ast.allocator);
        let member = Expression::StaticMemberExpression(ctx.ast.alloc(StaticMemberExpression {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            object: recv_cloned,
            property: IdentifierName {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                name: ctx.ast.ident(&info.method),
            },
            optional: false,
        }));
        let mut arguments = ctx.ast.vec();
        arguments.push(oxc_ast::ast::Argument::from(arg_cloned));
        Some(Expression::CallExpression(ctx.ast.alloc(CallExpression {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            callee: member,
            arguments,
            optional: false,
            type_arguments: None,
            pure: false,
        })))
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for MethodCallForwarderInliner {
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        if let Expression::CallExpression(call) = expr
            && let Some(inlined) = self.try_inline(call, ctx)
        {
            *expr = inlined;
        }
    }

    fn exit_statement(&mut self, stmt: &mut Statement<'a>, ctx: &mut Ctx<'a>) {
        if let Statement::FunctionDeclaration(func) = stmt
            && let Some(id) = &func.id
            && self.forwarders.contains_key(id.name.as_str())
        {
            eprintln!("[AST/method-fwd] remove function {}", id.name.as_str());
            *stmt = Statement::EmptyStatement(ctx.ast.alloc(EmptyStatement {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
            }));
            return;
        }
        if let Statement::VariableDeclaration(decl) = stmt {
            let all_are_forwarders = !decl.declarations.is_empty()
                && decl.declarations.iter().all(|d| {
                    let BindingPattern::BindingIdentifier(id) = &d.id else {
                        return false;
                    };
                    let Some(Expression::FunctionExpression(_)) = &d.init else {
                        return false;
                    };
                    self.forwarders.contains_key(id.name.as_str())
                });
            if all_are_forwarders {
                eprintln!("[AST/method-fwd] remove var declaration");
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
        let mut collector = MethodCallForwarderCollector::new();
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);
        traverse_mut_with_ctx(&mut collector, &mut program, &mut ctx);
        let fwds = collector.into_forwarders();
        if !fwds.is_empty() {
            let mut inliner = MethodCallForwarderInliner::new(fwds);
            let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
            traverse_mut_with_ctx(&mut inliner, &mut program, &mut ctx);
        }
        Codegen::new().build(&program).code
    }

    #[test]
    fn inlines_fn_decl_charcodeat() {
        let out = run("function J(a, b) { return a.charCodeAt(b); } var r = J(s, 3);");
        assert!(out.contains("s.charCodeAt(3)"), "got: {out}");
        assert!(!out.contains("function J"), "forwarder must be removed: {out}");
    }

    #[test]
    fn inlines_var_method_forwarder() {
        let out = run("var idx = function(a, b) { return a.indexOf(b); }; var r = idx(arr, x);");
        assert!(out.contains("arr.indexOf(x)"), "got: {out}");
        assert!(!out.contains("var idx"), "declaration must be removed: {out}");
    }

    #[test]
    fn leaves_swapped_param_alone() {
        let out = run("function S(a, b) { return b.charCodeAt(a); } S(1, s);");
        assert!(out.contains("function S"), "must keep (params swapped): {out}");
    }

    #[test]
    fn leaves_multi_arg_method_alone() {
        let out = run("function M(a, b) { return a.substring(0, b); } M(s, 5);");
        assert!(out.contains("function M"), "must keep (2 method args): {out}");
    }

    #[test]
    fn leaves_single_param_alone() {
        let out = run("function U(a) { return a.toUpperCase(); } U(s);");
        assert!(
            out.contains("function U"),
            "must keep (1-param is not our pattern): {out}"
        );
    }

    #[test]
    fn leaves_non_ident_receiver_alone() {
        let out = run("function K(a, b) { return other.charCodeAt(b); } K(x, 2);");
        assert!(out.contains("function K"), "must keep (receiver is not param0): {out}");
    }

    #[test]
    fn preserves_receiver_expression_at_call_site() {
        let out = run("function J(a, b) { return a.charCodeAt(b); } var r = J(getStr(), i + 1);");
        assert!(
            out.contains("getStr().charCodeAt(i + 1)") || out.contains("getStr().charCodeAt(i+1)"),
            "got: {out}"
        );
    }
}
