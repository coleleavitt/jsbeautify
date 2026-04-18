//! 1-arg lookup forwarder inliner.
//!
//! Detects functions of the exact shape
//!
//! ```text
//! function bI(x) { return <accessor>()[x]; }
//! var bI = function(x) { return <accessor>()[x]; };
//! ```
//!
//! where `<accessor>` is a plain identifier (ideally a self-init accessor).
//! Every call site `bI(y)` is rewritten to `<accessor>()[y]` and the forwarder
//! declaration removed.
//!
//! Akamai BMP uses this idiom heavily: `bI`, `QI`, `fG`, `jU`, `hY`, `sK`, `mU`,
//! `qm`, `WW` — all tiny 1-arg forwarders to `x19()` / `Y49()` lookup tables.
//! After inlining, call sites become `x19()[index]` which reads as ordinary
//! array indexing.

use oxc_allocator::CloneIn;
use oxc_ast::ast::{
    BindingPattern, CallExpression, ComputedMemberExpression, EmptyStatement, Expression, Function,
    IdentifierReference, Statement, VariableDeclarator,
};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::FxHashMap;
use std::cell::Cell;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

#[derive(Debug, Clone)]
pub struct LookupForwarderInfo {
    pub accessor_name: String,
}

pub struct LookupForwarderCollector {
    forwarders: FxHashMap<String, LookupForwarderInfo>,
}

impl LookupForwarderCollector {
    #[must_use]
    pub fn new() -> Self {
        Self {
            forwarders: FxHashMap::default(),
        }
    }

    #[must_use]
    pub fn forwarders(self) -> FxHashMap<String, LookupForwarderInfo> {
        self.forwarders
    }

    pub fn extract_info(func: &Function<'_>) -> Option<LookupForwarderInfo> {
        if func.r#async || func.generator {
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
        })
    }
}

impl Default for LookupForwarderCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for LookupForwarderCollector {
    fn enter_function(&mut self, func: &mut Function<'a>, _ctx: &mut Ctx<'a>) {
        if let Some(id) = &func.id
            && let Some(info) = Self::extract_info(func)
        {
            eprintln!(
                "[AST/lookup-fwd] found  function {}(x) {{ return {}()[x]; }}",
                id.name.as_str(),
                info.accessor_name
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
                "[AST/lookup-fwd] found  var {} = function(x) {{ return {}()[x]; }};",
                id.name.as_str(),
                info.accessor_name
            );
            self.forwarders.insert(id.name.as_str().to_string(), info);
        }
    }
}

pub struct LookupForwarderInliner {
    forwarders: FxHashMap<String, LookupForwarderInfo>,
    inlined: usize,
}

impl LookupForwarderInliner {
    #[must_use]
    pub fn new(forwarders: FxHashMap<String, LookupForwarderInfo>) -> Self {
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
        if call.arguments.len() != 1 {
            return None;
        }
        let arg = call.arguments[0].as_expression()?;
        if self.inlined < 10 {
            eprintln!(
                "[AST/lookup-fwd] inline {}(x) -> {}()[x]",
                ident.name.as_str(),
                info.accessor_name
            );
        }
        self.inlined += 1;

        let accessor_callee = Expression::Identifier(ctx.ast.alloc(IdentifierReference {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            name: ctx.ast.ident(&info.accessor_name),
            reference_id: Cell::default(),
        }));
        let accessor_call = Expression::CallExpression(ctx.ast.alloc(CallExpression {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            callee: accessor_callee,
            arguments: ctx.ast.vec(),
            optional: false,
            type_arguments: None,
            pure: false,
        }));
        let index_expr = arg.clone_in_with_semantic_ids(ctx.ast.allocator);
        Some(Expression::ComputedMemberExpression(ctx.ast.alloc(
            ComputedMemberExpression {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                object: accessor_call,
                expression: index_expr,
                optional: false,
            },
        )))
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for LookupForwarderInliner {
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
            eprintln!("[AST/lookup-fwd] remove function {}", id.name.as_str());
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
                    let Some(Expression::FunctionExpression(func)) = &d.init else {
                        return false;
                    };
                    self.forwarders.contains_key(id.name.as_str())
                        && LookupForwarderCollector::extract_info(func).is_some()
                });
            if all_are_forwarders {
                eprintln!("[AST/lookup-fwd] remove var declaration");
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
        let mut collector = LookupForwarderCollector::new();
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);
        traverse_mut_with_ctx(&mut collector, &mut program, &mut ctx);
        let fwds = collector.forwarders();
        if !fwds.is_empty() {
            let mut inliner = LookupForwarderInliner::new(fwds);
            let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
            traverse_mut_with_ctx(&mut inliner, &mut program, &mut ctx);
        }
        Codegen::new().build(&program).code
    }

    #[test]
    fn inlines_fn_declaration_forwarder() {
        let out = run("function bI(x) { return x19()[x]; } var r = bI(5);");
        assert!(out.contains("x19()[5]"), "got: {out}");
        assert!(!out.contains("function bI"), "forwarder must be removed: {out}");
    }

    #[test]
    fn inlines_var_forwarder() {
        let out = run("var bI = function(x) { return x19()[x]; }; var r = bI(y);");
        assert!(out.contains("x19()[y]"), "got: {out}");
        assert!(!out.contains("var bI"), "declaration must be removed: {out}");
    }

    #[test]
    fn leaves_multi_arg_alone() {
        let out = run("function f(x, y) { return x19()[x]; } f(1, 2);");
        assert!(out.contains("function f"), "two-arg fn must be kept: {out}");
    }

    #[test]
    fn leaves_non_member_return_alone() {
        let out = run("function f(x) { return x + 1; } f(5);");
        assert!(out.contains("function f"), "must keep: {out}");
    }

    #[test]
    fn leaves_accessor_with_args_alone() {
        let out = run("function f(x) { return acc(1)[x]; } f(5);");
        assert!(out.contains("function f"), "must keep (accessor takes args): {out}");
    }

    #[test]
    fn does_not_remove_same_name_var_with_different_shape() {
        // Inner function Xt is a lookup forwarder; outer var Xt is a 2-arg operator proxy.
        // The inliner must NOT remove the outer var Xt declaration.
        let code = r#"
            var Xt = function(a, b) { return a + b; };
            (function Ql() {
                function Xt(x) { return Tl()[x]; }
                var r = Xt(5);
            })();
            var z = Xt(1, 2);
        "#;
        let out = run(code);
        assert!(
            out.contains("var Xt"),
            "outer var Xt (operator proxy) must be preserved: {out}"
        );
    }
}
