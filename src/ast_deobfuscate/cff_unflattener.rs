//! CFF (Control-Flow-Flattening) unflattener.
//!
//! Consumes a [`DispatcherMap`] (from [`super::dispatcher_detector`]) and inlines
//! dispatcher call sites by replacing them with IIFEs containing the cloned
//! case body.
//!
//! ```text
//! // Before:
//! P6(LN, [kA])
//!
//! // After:
//! (function(pE) {
//!   var dY = pE[NF];
//!   dY[dY[JF](rb)] = function() { ... };
//!   P6(LN, [dY]);
//! }([kA]))
//! ```
//!
//! The IIFE approach preserves scoping, `break` semantics, and `return`
//! semantics without needing to resolve `args[INDEX]` references.

use oxc_allocator::{CloneIn, Vec as OxcVec};
use oxc_ast::ast::{
    Argument, BindingIdentifier, BindingPattern, CallExpression, Expression, FormalParameter, FormalParameterKind,
    FormalParameters, Function, FunctionBody, FunctionType, Program, Statement, VariableDeclarator,
};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::FxHashMap;
use std::cell::Cell;

use super::dispatcher_detector::DispatcherMap;
use super::state::DeobfuscateState;

type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

type CaseBodyMap<'a> = FxHashMap<(String, String), OxcVec<'a, Statement<'a>>>;

struct DispatcherMeta {
    args_param: String,
}

pub fn collect_case_bodies<'a>(
    program: &Program<'a>,
    dispatchers: &DispatcherMap,
    alloc: &'a oxc_allocator::Allocator,
) -> CaseBodyMap<'a> {
    let mut bodies = FxHashMap::default();
    scan_stmts(&program.body, dispatchers, &mut bodies, alloc);
    bodies
}

fn scan_stmts<'a>(
    stmts: &OxcVec<'a, Statement<'a>>,
    dispatchers: &DispatcherMap,
    bodies: &mut CaseBodyMap<'a>,
    alloc: &'a oxc_allocator::Allocator,
) {
    for stmt in stmts {
        scan_stmt(stmt, dispatchers, bodies, alloc);
    }
}

fn scan_stmt<'a>(
    stmt: &Statement<'a>,
    dispatchers: &DispatcherMap,
    bodies: &mut CaseBodyMap<'a>,
    alloc: &'a oxc_allocator::Allocator,
) {
    match stmt {
        Statement::FunctionDeclaration(func) => {
            if let Some(id) = &func.id {
                let name = id.name.as_str();
                if let Some(info) = dispatchers.get(name) {
                    extract_case_bodies(func, &info.name, dispatchers, bodies, alloc);
                }
            }
            if let Some(body) = &func.body {
                scan_stmts(&body.statements, dispatchers, bodies, alloc);
            }
        }
        Statement::VariableDeclaration(decl) => {
            for d in &decl.declarations {
                scan_var_declarator(d, dispatchers, bodies, alloc);
            }
        }
        Statement::ExpressionStatement(es) => {
            if let Expression::CallExpression(call) = &es.expression {
                let callee = match &call.callee {
                    Expression::ParenthesizedExpression(p) => &p.expression,
                    other => other,
                };
                if let Expression::FunctionExpression(func) = callee {
                    if let Some(body) = &func.body {
                        scan_stmts(&body.statements, dispatchers, bodies, alloc);
                    }
                }
            }
        }
        Statement::BlockStatement(b) => scan_stmts(&b.body, dispatchers, bodies, alloc),
        Statement::IfStatement(ifs) => {
            scan_stmt(&ifs.consequent, dispatchers, bodies, alloc);
            if let Some(alt) = &ifs.alternate {
                scan_stmt(alt, dispatchers, bodies, alloc);
            }
        }
        Statement::TryStatement(t) => {
            scan_stmts(&t.block.body, dispatchers, bodies, alloc);
            if let Some(h) = &t.handler {
                scan_stmts(&h.body.body, dispatchers, bodies, alloc);
            }
            if let Some(f) = &t.finalizer {
                scan_stmts(&f.body, dispatchers, bodies, alloc);
            }
        }
        Statement::ForStatement(f) => scan_stmt(&f.body, dispatchers, bodies, alloc),
        Statement::WhileStatement(w) => scan_stmt(&w.body, dispatchers, bodies, alloc),
        Statement::DoWhileStatement(d) => scan_stmt(&d.body, dispatchers, bodies, alloc),
        Statement::SwitchStatement(s) => {
            for case in &s.cases {
                scan_stmts(&case.consequent, dispatchers, bodies, alloc);
            }
        }
        _ => {}
    }
}

fn scan_var_declarator<'a>(
    declarator: &VariableDeclarator<'a>,
    dispatchers: &DispatcherMap,
    bodies: &mut CaseBodyMap<'a>,
    alloc: &'a oxc_allocator::Allocator,
) {
    let BindingPattern::BindingIdentifier(var_id) = &declarator.id else {
        return;
    };
    let Some(Expression::FunctionExpression(func)) = &declarator.init else {
        return;
    };
    let var_name = var_id.name.as_str();
    if let Some(info) = dispatchers.get(var_name) {
        extract_case_bodies(func, &info.name, dispatchers, bodies, alloc);
    }
}

fn extract_case_bodies<'a>(
    func: &Function<'a>,
    dispatcher_name: &str,
    dispatchers: &DispatcherMap,
    bodies: &mut CaseBodyMap<'a>,
    alloc: &'a oxc_allocator::Allocator,
) {
    let Some(body) = &func.body else { return };
    if body.statements.len() != 1 {
        return;
    }
    let Statement::SwitchStatement(switch) = &body.statements[0] else {
        return;
    };
    let info = match dispatchers.get(dispatcher_name) {
        Some(i) => i,
        None => return,
    };

    for case in &switch.cases {
        let Some(Expression::Identifier(label_id)) = &case.test else {
            continue;
        };
        let label = label_id.name.as_str();
        if !info.cases.contains_key(label) {
            continue;
        }
        let mut stmts: OxcVec<'a, Statement<'a>> = OxcVec::new_in(alloc);
        for s in &case.consequent {
            if matches!(s, Statement::BreakStatement(_)) {
                continue;
            }
            stmts.push(s.clone_in_with_semantic_ids(alloc));
        }
        bodies.insert((dispatcher_name.to_string(), label.to_string()), stmts);
    }
}

pub struct CffUnflattener<'a> {
    dispatchers: DispatcherMap,
    case_bodies: CaseBodyMap<'a>,
    meta: FxHashMap<String, DispatcherMeta>,
    inlined: usize,
}

impl<'a> CffUnflattener<'a> {
    #[must_use]
    pub fn new(dispatchers: DispatcherMap, case_bodies: CaseBodyMap<'a>) -> Self {
        let meta: FxHashMap<String, DispatcherMeta> = dispatchers
            .iter()
            .map(|(name, info)| {
                (
                    name.clone(),
                    DispatcherMeta {
                        args_param: info.args_param.clone(),
                    },
                )
            })
            .collect();
        Self {
            dispatchers,
            case_bodies,
            meta,
            inlined: 0,
        }
    }

    #[must_use]
    pub const fn inlined(&self) -> usize {
        self.inlined
    }

    fn make_iife(
        args_param_name: &str,
        body_stmts: &OxcVec<'a, Statement<'a>>,
        call_site_arg: &Expression<'a>,
        ctx: &Ctx<'a>,
    ) -> Expression<'a> {
        let alloc = ctx.ast.allocator;

        let mut stmts: OxcVec<'a, Statement<'a>> = ctx.ast.vec();
        for s in body_stmts {
            stmts.push(s.clone_in_with_semantic_ids(alloc));
        }

        let fn_body = FunctionBody {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            directives: ctx.ast.vec(),
            statements: stmts,
        };

        let binding_id = BindingIdentifier {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            name: ctx.ast.ident(args_param_name),
            symbol_id: Cell::default(),
        };
        let param = FormalParameter {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            decorators: ctx.ast.vec(),
            pattern: BindingPattern::BindingIdentifier(ctx.ast.alloc(binding_id)),
            type_annotation: None,
            initializer: None,
            optional: false,
            accessibility: None,
            readonly: false,
            r#override: false,
        };
        let mut items: OxcVec<'a, FormalParameter<'a>> = ctx.ast.vec();
        items.push(param);
        let params = FormalParameters {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            kind: FormalParameterKind::FormalParameter,
            items,
            rest: None,
        };

        let func = Function {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            r#type: FunctionType::FunctionExpression,
            id: None,
            generator: false,
            r#async: false,
            declare: false,
            type_parameters: None,
            this_param: None,
            params: ctx.ast.alloc(params),
            return_type: None,
            body: Some(ctx.ast.alloc(fn_body)),
            scope_id: Cell::new(None),
            pure: false,
            pife: false,
        };
        let callee = Expression::FunctionExpression(ctx.ast.alloc(func));

        let arg_clone = call_site_arg.clone_in_with_semantic_ids(alloc);
        let mut arguments: OxcVec<'a, Argument<'a>> = ctx.ast.vec();
        arguments.push(Argument::from(arg_clone));

        Expression::CallExpression(ctx.ast.alloc(CallExpression {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            callee,
            arguments,
            optional: false,
            type_arguments: None,
            pure: false,
        }))
    }

    fn try_inline_direct(&mut self, call: &CallExpression<'a>, ctx: &Ctx<'a>) -> Option<Expression<'a>> {
        let Expression::Identifier(callee_id) = &call.callee else {
            return None;
        };
        let name = callee_id.name.as_str();
        if !self.dispatchers.contains_key(name) {
            return None;
        }
        if call.arguments.len() != 2 {
            return None;
        }

        let Some(state_expr) = call.arguments[0].as_expression() else {
            return None;
        };
        let Expression::Identifier(state_id) = state_expr else {
            return None;
        };
        let state_label = state_id.name.as_str();

        let key = (name.to_string(), state_label.to_string());
        if !self.case_bodies.contains_key(&key) {
            return None;
        }

        let Some(args_expr) = call.arguments[1].as_expression() else {
            return None;
        };

        let meta = self.meta.get(name)?;
        let body = self.case_bodies.get(&key)?;

        if self.inlined < 10 {
            eprintln!("[CFF] inlining {}({}, [...]) at call site", name, state_label);
        }
        self.inlined += 1;

        Some(Self::make_iife(&meta.args_param, body, args_expr, ctx))
    }

    fn try_inline_call_this(&mut self, call: &CallExpression<'a>, ctx: &Ctx<'a>) -> Option<Expression<'a>> {
        let Expression::StaticMemberExpression(sme) = &call.callee else {
            return None;
        };
        if sme.property.name.as_str() != "call" {
            return None;
        }
        let Expression::Identifier(obj_id) = &sme.object else {
            return None;
        };
        let name = obj_id.name.as_str();
        if !self.dispatchers.contains_key(name) {
            return None;
        }
        if call.arguments.len() != 3 {
            return None;
        }

        let Some(first) = call.arguments[0].as_expression() else {
            return None;
        };
        if !matches!(first, Expression::ThisExpression(_)) {
            return None;
        }

        let Some(state_expr) = call.arguments[1].as_expression() else {
            return None;
        };
        let Expression::Identifier(state_id) = state_expr else {
            return None;
        };
        let state_label = state_id.name.as_str();

        let key = (name.to_string(), state_label.to_string());
        if !self.case_bodies.contains_key(&key) {
            return None;
        }

        let Some(args_expr) = call.arguments[2].as_expression() else {
            return None;
        };

        let meta = self.meta.get(name)?;
        let body = self.case_bodies.get(&key)?;

        if self.inlined < 10 {
            eprintln!(
                "[CFF] inlining {}.call(this, {}, [...]) at call site",
                name, state_label
            );
        }
        self.inlined += 1;

        Some(Self::make_iife(&meta.args_param, body, args_expr, ctx))
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for CffUnflattener<'a> {
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        if let Expression::CallExpression(call) = expr {
            if let Some(replacement) = self.try_inline_direct(call, ctx) {
                *expr = replacement;
                return;
            }
            if let Some(replacement) = self.try_inline_call_this(call, ctx) {
                *expr = replacement;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast_deobfuscate::dispatcher_detector::DispatcherDetector;
    use oxc_allocator::Allocator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_semantic::SemanticBuilder;
    use oxc_span::SourceType;
    use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

    fn run(code: &str) -> (String, usize) {
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let mut program = ret.program;

        let detector = DispatcherDetector::new();
        let dispatchers = detector.detect(&program);
        if dispatchers.is_empty() {
            return (Codegen::new().build(&program).code, 0);
        }

        let case_bodies = collect_case_bodies(&program, &dispatchers, &allocator);
        let mut unflattener = CffUnflattener::new(dispatchers, case_bodies);

        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        traverse_mut_with_ctx(&mut unflattener, &mut program, &mut ctx);

        let inlined = unflattener.inlined();
        (Codegen::new().build(&program).code, inlined)
    }

    #[test]
    fn inlines_simple_dispatcher_call() {
        let code = r#"
            function D(s, a) {
                switch (s) {
                    case X: { return a[0] + 1; } break;
                }
            }
            var r = D(X, [5]);
        "#;
        let (out, inlined) = run(code);
        assert_eq!(inlined, 1, "should inline 1 call, got: {out}");
        assert!(
            out.contains("function(a)"),
            "should have IIFE with args param 'a': {out}"
        );
        assert!(
            !out.contains("D(X,") && !out.contains("D(X, "),
            "original call should be replaced: {out}"
        );
    }

    #[test]
    fn preserves_non_literal_state() {
        let code = r#"
            function D(s, a) {
                switch (s) {
                    case X: { return a[0]; } break;
                }
            }
            D(someVar, [5]);
        "#;
        let (out, inlined) = run(code);
        assert_eq!(inlined, 0, "should not inline computed state: {out}");
        assert!(out.contains("D(someVar"), "call should be preserved: {out}");
    }

    #[test]
    fn preserves_unknown_dispatcher() {
        let code = r#"
            function D(s, a) {
                switch (s) {
                    case X: { return a[0]; } break;
                }
            }
            unknown(X, [5]);
        "#;
        let (out, inlined) = run(code);
        assert_eq!(inlined, 0, "should not inline unknown function: {out}");
        assert!(out.contains("unknown(X"), "unknown call should be preserved: {out}");
    }

    #[test]
    fn handles_call_this_form() {
        let code = r#"
            function D(s, a) {
                switch (s) {
                    case X: { return a[0] + 1; } break;
                }
            }
            D.call(this, X, [5]);
        "#;
        let (out, inlined) = run(code);
        assert_eq!(inlined, 1, "should inline .call(this,...) form: {out}");
        assert!(out.contains("function(a)"), "should have IIFE with args param: {out}");
    }

    #[test]
    fn handles_multiple_cases() {
        let code = r#"
            function D(s, a) {
                switch (s) {
                    case A: { return a[0] + 1; } break;
                    case B: { return a[0] * 2; } break;
                    case C: { return a[0] - 3; } break;
                }
            }
            D(A, [1]);
            D(B, [2]);
            D(C, [3]);
        "#;
        let (out, inlined) = run(code);
        assert_eq!(inlined, 3, "should inline all 3 calls: {out}");
    }

    #[test]
    fn counts_inlines() {
        let code = r#"
            function D(s, a) {
                switch (s) {
                    case X: { foo(); } break;
                    case Y: { bar(); } break;
                }
            }
            D(X, [1]);
            D(Y, [2]);
            D(X, [3]);
        "#;
        let (_, inlined) = run(code);
        assert_eq!(inlined, 3);
    }

    #[test]
    fn iife_contains_case_body() {
        let code = r#"
            function D(s, a) {
                switch (s) {
                    case X: { var t = a[0]; foo(t); } break;
                }
            }
            D(X, [42]);
        "#;
        let (out, inlined) = run(code);
        assert_eq!(inlined, 1);
        assert!(out.contains("foo(t)"), "case body should be in IIFE: {out}");
        assert!(out.contains("var t = a[0]"), "var decl should be in IIFE: {out}");
    }

    #[test]
    fn skips_wrong_arg_count() {
        let code = r#"
            function D(s, a) {
                switch (s) {
                    case X: { return a[0]; } break;
                }
            }
            D(X);
            D(X, [1], extra);
        "#;
        let (out, inlined) = run(code);
        assert_eq!(inlined, 0, "wrong arg count should not inline: {out}");
    }

    #[test]
    fn handles_nested_in_iife() {
        let code = r#"
            (function() {
                function D(s, a) {
                    switch (s) {
                        case X: { return a[0]; } break;
                    }
                }
                D(X, [1]);
            })();
        "#;
        let (out, inlined) = run(code);
        assert_eq!(inlined, 1, "should inline inside IIFE wrapper: {out}");
    }
}
