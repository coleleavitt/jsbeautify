//! Trampoline inliner for state-machine dispatchers.
//!
//! Akamai BMP (and similar Jscrambler-based obfuscators) expand every logical
//! function into a tiny trampoline into a central dispatcher:
//!
//! ```text
//! var xB = function() { return LT.apply(this, [S, arguments]); };
//! function jQ()         { return JA.apply(this, [Qz, arguments]); }
//! ```
//!
//! where `LT` / `JA` / `hY` / `SY` / `ZE` / `tQ` / … are top-level interpreter
//! functions that dispatch on their first argument (a state constant such as
//! `S`, `Qz`, `ll`, `E5`, …). Hundreds of call sites `xB(a, b, c)` are
//! semantically equivalent to `LT.call(this, S, [a, b, c])`.
//!
//! This pass:
//! 1. Collects trampolines. Recognised shapes:
//!    * `function NAME() { return DISPATCH.apply(this, [STATE, arguments]); }`
//!    * `var NAME = function() { return DISPATCH.apply(this, [STATE, arguments]); };`
//!    * `var NAME = function NAME2() { return DISPATCH.apply(this, [STATE, arguments]); };`
//!    DISPATCH must be a plain identifier; STATE must be a plain identifier.
//! 2. At every call site `NAME(<args>)` (identifier call, not member call), rewrites to
//!    `DISPATCH.call(this, STATE, [<args>])`.
//! 3. Removes the trampoline declaration.
//!
//! The `this` is preserved by using `.call(this, …)`. The `arguments` object
//! becomes an explicit array-literal argument — the dispatchers already
//! index into their 2nd parameter as `arg[idx]`, so array-literal works.

use oxc_allocator::{CloneIn, Vec as OxcVec};
use oxc_ast::ast::{
    Argument, ArrayExpression, ArrayExpressionElement, BindingPattern, CallExpression, EmptyStatement, Expression,
    Function, IdentifierName, IdentifierReference, Statement, StaticMemberExpression, ThisExpression,
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
pub struct TrampolineInfo {
    pub dispatcher: String,
    pub state: String,
}

pub struct TrampolineCollector {
    trampolines: FxHashMap<String, TrampolineInfo>,
}

impl TrampolineCollector {
    #[must_use]
    pub fn new() -> Self {
        Self {
            trampolines: FxHashMap::default(),
        }
    }

    #[must_use]
    pub fn into_trampolines(self) -> FxHashMap<String, TrampolineInfo> {
        self.trampolines
    }

    fn extract_info(func: &Function<'_>) -> Option<TrampolineInfo> {
        if func.r#async || func.generator {
            return None;
        }
        if !func.params.items.is_empty() {
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
        let Some(first) = call.arguments[0].as_expression() else {
            return None;
        };
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
        if second.name.as_str() != "arguments" {
            return None;
        }
        Some(TrampolineInfo {
            dispatcher: dispatcher.name.as_str().to_string(),
            state: state.name.as_str().to_string(),
        })
    }
}

impl Default for TrampolineCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for TrampolineCollector {
    fn enter_function(&mut self, func: &mut Function<'a>, _ctx: &mut Ctx<'a>) {
        if let Some(id) = &func.id
            && let Some(info) = Self::extract_info(func)
        {
            eprintln!(
                "[AST/tramp] found  function {}() -> {}.apply(this, [{}, arguments])",
                id.name.as_str(),
                info.dispatcher,
                info.state
            );
            self.trampolines.insert(id.name.as_str().to_string(), info);
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
                "[AST/tramp] found  var {} = function() -> {}.apply(this, [{}, arguments])",
                id.name.as_str(),
                info.dispatcher,
                info.state
            );
            self.trampolines.insert(id.name.as_str().to_string(), info);
        }
    }
}

pub struct TrampolineInliner {
    trampolines: FxHashMap<String, TrampolineInfo>,
    inlined: usize,
}

impl TrampolineInliner {
    #[must_use]
    pub fn new(trampolines: FxHashMap<String, TrampolineInfo>) -> Self {
        Self {
            trampolines,
            inlined: 0,
        }
    }

    #[must_use]
    pub const fn inlined(&self) -> usize {
        self.inlined
    }

    fn make_call_expr<'a>(
        dispatcher: &str,
        state: &str,
        call_args: &OxcVec<'a, Argument<'a>>,
        ctx: &Ctx<'a>,
    ) -> Expression<'a> {
        let dispatcher_ident = Expression::Identifier(ctx.ast.alloc(IdentifierReference {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            name: ctx.ast.ident(dispatcher),
            reference_id: Cell::default(),
        }));
        let call_member = Expression::StaticMemberExpression(ctx.ast.alloc(StaticMemberExpression {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            object: dispatcher_ident,
            property: IdentifierName {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                name: ctx.ast.ident("call"),
            },
            optional: false,
        }));

        let this_arg = Expression::ThisExpression(ctx.ast.alloc(ThisExpression {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
        }));
        let state_arg = Expression::Identifier(ctx.ast.alloc(IdentifierReference {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            name: ctx.ast.ident(state),
            reference_id: Cell::default(),
        }));

        let mut array_elems: OxcVec<'a, ArrayExpressionElement<'a>> = ctx.ast.vec();
        for arg in call_args {
            if let Some(e) = arg.as_expression() {
                array_elems.push(ArrayExpressionElement::from(
                    e.clone_in_with_semantic_ids(ctx.ast.allocator),
                ));
            } else {
                let cloned_arg: Argument<'a> = arg.clone_in_with_semantic_ids(ctx.ast.allocator);
                array_elems.push(cloned_arg.into());
            }
        }
        let args_array = Expression::ArrayExpression(ctx.ast.alloc(ArrayExpression {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            elements: array_elems,
        }));

        let mut new_arguments: OxcVec<'a, Argument<'a>> = ctx.ast.vec();
        new_arguments.push(Argument::from(this_arg));
        new_arguments.push(Argument::from(state_arg));
        new_arguments.push(Argument::from(args_array));

        Expression::CallExpression(ctx.ast.alloc(CallExpression {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            callee: call_member,
            arguments: new_arguments,
            optional: false,
            type_arguments: None,
            pure: false,
        }))
    }

    fn try_inline<'a>(&mut self, call: &CallExpression<'a>, ctx: &mut Ctx<'a>) -> Option<Expression<'a>> {
        let Expression::Identifier(ident) = &call.callee else {
            return None;
        };
        let info = self.trampolines.get(ident.name.as_str())?;
        if self.inlined < 10 {
            eprintln!(
                "[AST/tramp] inline {}({} args) -> {}.call(this, {}, [...])",
                ident.name.as_str(),
                call.arguments.len(),
                info.dispatcher,
                info.state
            );
        }
        self.inlined += 1;
        Some(Self::make_call_expr(
            &info.dispatcher,
            &info.state,
            &call.arguments,
            ctx,
        ))
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for TrampolineInliner {
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
            && self.trampolines.contains_key(id.name.as_str())
        {
            eprintln!("[AST/tramp] remove function {}", id.name.as_str());
            *stmt = Statement::EmptyStatement(ctx.ast.alloc(EmptyStatement {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
            }));
            return;
        }
        if let Statement::VariableDeclaration(decl) = stmt {
            let all_are_trampolines = !decl.declarations.is_empty()
                && decl.declarations.iter().all(|d| {
                    let BindingPattern::BindingIdentifier(id) = &d.id else {
                        return false;
                    };
                    let Some(Expression::FunctionExpression(_)) = &d.init else {
                        return false;
                    };
                    self.trampolines.contains_key(id.name.as_str())
                });
            if all_are_trampolines {
                eprintln!("[AST/tramp] remove var declaration");
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
        let mut collector = TrampolineCollector::new();
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);
        traverse_mut_with_ctx(&mut collector, &mut program, &mut ctx);
        let trampolines = collector.into_trampolines();
        if !trampolines.is_empty() {
            let mut inliner = TrampolineInliner::new(trampolines);
            let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
            traverse_mut_with_ctx(&mut inliner, &mut program, &mut ctx);
        }
        Codegen::new().build(&program).code
    }

    #[test]
    fn inlines_var_trampoline() {
        let out = run("var xB = function() { return LT.apply(this, [S, arguments]); }; xB(1, 2);");
        assert!(!out.contains("var xB"), "declaration must be removed: {out}");
        assert!(
            out.contains("LT.call(this, S, [1, 2])") || out.contains("LT.call(this,S,[1,2])"),
            "got: {out}"
        );
    }

    #[test]
    fn inlines_function_decl_trampoline() {
        let out = run("function jQ() { return JA.apply(this, [Qz, arguments]); } jQ();");
        assert!(!out.contains("function jQ"), "declaration must be removed: {out}");
        assert!(
            out.contains("JA.call(this, Qz, [])") || out.contains("JA.call(this,Qz,[])"),
            "got: {out}"
        );
    }

    #[test]
    fn inlines_many_call_sites() {
        let out = run("var xB = function() { return LT.apply(this, [S, arguments]); }; xB(a); xB(b, c); xB();");
        assert!(!out.contains("var xB"));
        let hits = out.matches("LT.call(this").count();
        assert_eq!(hits, 3, "all 3 sites must be rewritten, got: {out}");
    }

    #[test]
    fn leaves_non_tramp_functions_alone() {
        let out = run("function f() { return g(1, 2); } f();");
        assert!(out.contains("function f"), "non-tramp must be kept: {out}");
    }

    #[test]
    fn leaves_non_this_apply_alone() {
        let out = run("var x = function() { return g.apply(null, [S, arguments]); }; x();");
        assert!(out.contains("var x"), "must keep (apply arg is null not this): {out}");
    }

    #[test]
    fn leaves_non_arguments_second_arg_alone() {
        let out = run("var x = function() { return g.apply(this, [S, [a,b]]); }; x();");
        assert!(out.contains("var x"), "must keep (2nd arg not 'arguments'): {out}");
    }
}
