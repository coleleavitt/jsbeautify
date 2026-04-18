//! Self-initialising accessor flattener.
//!
//! Rewrites the Akamai-BMP / Jscrambler idiom
//!
//! ```text
//! function hx() {
//!     var x = <init>;           // [] | {} | new Object() | [].entries() | …
//!     hx = function () { return x; };
//!     return x;
//! }
//! ```
//!
//! into the equivalent but far more readable:
//!
//! ```text
//! function hx() { return x; }
//! ```
//!
//! with a hoisted `var x = <init>;` inserted at the start of the enclosing
//! function-scope or program body. The rewrite preserves the cached-singleton
//! semantics — every call to `hx()` returns the same object — while removing
//! the self-reassignment that makes the code unreadable.
//!
//! We only rewrite accessors whose initialiser is one of a whitelist of pure,
//! side-effect-free expressions (`[]`, `{}`, `new Object()`, `new Array()`,
//! `Object.create(...)`, `[].entries()` / `[].keys()` / `[].values()`).
//! Anything else (call to user function, arithmetic, etc.) is left alone.

use oxc_allocator::Vec as OxcVec;
use oxc_ast::ast::{
    BindingIdentifier, BindingPattern, Expression, Function, Program, ReturnStatement, Statement, VariableDeclaration,
    VariableDeclarationKind, VariableDeclarator,
};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use oxc_traverse::{Traverse, TraverseCtx};
use std::cell::Cell;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InitKind {
    EmptyArray,
    EmptyObject,
    NewObject,
    NewArray,
    ObjectCreateNull,
    ObjectCreateProto,
    ArrayIter(&'static str),
}

fn classify_init<'a>(expr: &Expression<'a>) -> Option<InitKind> {
    match expr {
        Expression::ArrayExpression(a) if a.elements.is_empty() => Some(InitKind::EmptyArray),
        Expression::ObjectExpression(o) if o.properties.is_empty() => Some(InitKind::EmptyObject),
        Expression::NewExpression(ne) => match &ne.callee {
            Expression::Identifier(id) if id.name.as_str() == "Object" && ne.arguments.is_empty() => {
                Some(InitKind::NewObject)
            }
            Expression::Identifier(id) if id.name.as_str() == "Array" && ne.arguments.is_empty() => {
                Some(InitKind::NewArray)
            }
            _ => None,
        },
        Expression::CallExpression(call) => {
            if let Expression::StaticMemberExpression(sme) = &call.callee {
                if let Expression::Identifier(obj) = &sme.object
                    && obj.name.as_str() == "Object"
                    && sme.property.name.as_str() == "create"
                    && call.arguments.len() == 1
                {
                    if matches!(call.arguments[0].as_expression(), Some(Expression::NullLiteral(_))) {
                        return Some(InitKind::ObjectCreateNull);
                    }
                    return Some(InitKind::ObjectCreateProto);
                }
                if let Expression::ArrayExpression(arr) = &sme.object
                    && arr.elements.is_empty()
                    && matches!(sme.property.name.as_str(), "entries" | "keys" | "values")
                    && call.arguments.is_empty()
                {
                    let name: &'static str = match sme.property.name.as_str() {
                        "entries" => "entries",
                        "keys" => "keys",
                        "values" => "values",
                        _ => unreachable!(),
                    };
                    return Some(InitKind::ArrayIter(name));
                }
            }
            None
        }
        Expression::ParenthesizedExpression(p) => classify_init(&p.expression),
        _ => None,
    }
}

fn is_self_init_accessor_named<'a>(func: &Function<'a>) -> Option<(String, InitKind)> {
    let name = func.id.as_ref()?.name.as_str().to_string();
    if func.params.items.len() != 0 {
        return None;
    }
    let body = func.body.as_ref()?;
    if body.statements.len() != 3 {
        return None;
    }

    let Statement::VariableDeclaration(decl) = &body.statements[0] else {
        return None;
    };
    if decl.declarations.len() != 1 {
        return None;
    }
    let local_name = match &decl.declarations[0].id {
        BindingPattern::BindingIdentifier(id) => id.name.as_str(),
        _ => return None,
    };
    let init_kind = classify_init(decl.declarations[0].init.as_ref()?)?;

    let Statement::ExpressionStatement(expr_stmt) = &body.statements[1] else {
        return None;
    };
    let Expression::AssignmentExpression(assign) = &expr_stmt.expression else {
        return None;
    };
    let target_ok = matches!(
        &assign.left,
        oxc_ast::ast::AssignmentTarget::AssignmentTargetIdentifier(id) if id.name.as_str() == name
    );
    if !target_ok {
        return None;
    }
    let Expression::FunctionExpression(reassigned) = &assign.right else {
        return None;
    };
    let Some(reassigned_body) = reassigned.body.as_ref() else {
        return None;
    };
    if reassigned_body.statements.len() != 1 {
        return None;
    }
    let Statement::ReturnStatement(ret) = &reassigned_body.statements[0] else {
        return None;
    };
    let Some(Expression::Identifier(ret_id)) = &ret.argument else {
        return None;
    };
    if ret_id.name.as_str() != local_name {
        return None;
    }

    let Statement::ReturnStatement(ret) = &body.statements[2] else {
        return None;
    };
    let Some(Expression::Identifier(ret_id)) = &ret.argument else {
        return None;
    };
    if ret_id.name.as_str() != local_name {
        return None;
    }

    Some((name, init_kind))
}

pub struct SelfInitAccessorFlattener {
    rewritten: usize,
    cache_vars_to_hoist: Vec<(String, InitKind)>,
}

impl SelfInitAccessorFlattener {
    #[must_use]
    pub fn new() -> Self {
        Self {
            rewritten: 0,
            cache_vars_to_hoist: Vec::new(),
        }
    }

    #[must_use]
    pub fn rewritten(&self) -> usize {
        self.rewritten
    }

    fn make_init_expr<'a>(kind: InitKind, ctx: &Ctx<'a>) -> Expression<'a> {
        use oxc_ast::ast::{ArrayExpression, CallExpression, NewExpression, NullLiteral, ObjectExpression};
        match kind {
            InitKind::EmptyArray => Expression::ArrayExpression(ctx.ast.alloc(ArrayExpression {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                elements: ctx.ast.vec(),
            })),
            InitKind::EmptyObject => Expression::ObjectExpression(ctx.ast.alloc(ObjectExpression {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                properties: ctx.ast.vec(),
            })),
            InitKind::NewObject | InitKind::NewArray => {
                let name = if matches!(kind, InitKind::NewObject) {
                    "Object"
                } else {
                    "Array"
                };
                let callee = Expression::Identifier(ctx.ast.alloc(oxc_ast::ast::IdentifierReference {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                    name: ctx.ast.ident(name),
                    reference_id: Cell::default(),
                }));
                Expression::NewExpression(ctx.ast.alloc(NewExpression {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                    callee,
                    arguments: ctx.ast.vec(),
                    type_arguments: None,
                    pure: false,
                }))
            }
            InitKind::ObjectCreateNull | InitKind::ObjectCreateProto => {
                let obj = Expression::Identifier(ctx.ast.alloc(oxc_ast::ast::IdentifierReference {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                    name: ctx.ast.ident("Object"),
                    reference_id: Cell::default(),
                }));
                let property = oxc_ast::ast::IdentifierName {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                    name: ctx.ast.ident("create"),
                };
                let callee = Expression::StaticMemberExpression(ctx.ast.alloc(oxc_ast::ast::StaticMemberExpression {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                    object: obj,
                    property,
                    optional: false,
                }));
                let arg_expr = if matches!(kind, InitKind::ObjectCreateNull) {
                    Expression::NullLiteral(ctx.ast.alloc(NullLiteral {
                        node_id: Cell::new(NodeId::DUMMY),
                        span: SPAN,
                    }))
                } else {
                    Expression::ObjectExpression(ctx.ast.alloc(ObjectExpression {
                        node_id: Cell::new(NodeId::DUMMY),
                        span: SPAN,
                        properties: ctx.ast.vec(),
                    }))
                };
                let mut args = ctx.ast.vec();
                args.push(oxc_ast::ast::Argument::from(arg_expr));
                Expression::CallExpression(ctx.ast.alloc(CallExpression {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                    callee,
                    arguments: args,
                    optional: false,
                    type_arguments: None,
                    pure: false,
                }))
            }
            InitKind::ArrayIter(method) => {
                let empty_arr = Expression::ArrayExpression(ctx.ast.alloc(ArrayExpression {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                    elements: ctx.ast.vec(),
                }));
                let property = oxc_ast::ast::IdentifierName {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                    name: ctx.ast.ident(method),
                };
                let callee = Expression::StaticMemberExpression(ctx.ast.alloc(oxc_ast::ast::StaticMemberExpression {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                    object: empty_arr,
                    property,
                    optional: false,
                }));
                Expression::CallExpression(ctx.ast.alloc(CallExpression {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                    callee,
                    arguments: ctx.ast.vec(),
                    optional: false,
                    type_arguments: None,
                    pure: false,
                }))
            }
        }
    }

    fn try_rewrite_function<'a>(func: &mut Function<'a>, ctx: &Ctx<'a>) -> Option<(String, InitKind)> {
        let (name, kind) = is_self_init_accessor_named(func)?;
        let cache_var = format!("__{name}_cache");

        let body = func.body.as_mut()?;
        let mut new_stmts = ctx.ast.vec();
        let return_stmt = Statement::ReturnStatement(ctx.ast.alloc(ReturnStatement {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            argument: Some(Expression::Identifier(ctx.ast.alloc(
                oxc_ast::ast::IdentifierReference {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                    name: ctx.ast.ident(&cache_var),
                    reference_id: Cell::default(),
                },
            ))),
        }));
        new_stmts.push(return_stmt);
        body.statements = new_stmts;
        Some((cache_var, kind))
    }

    fn hoist_cache_vars<'a>(&mut self, statements: &mut OxcVec<'a, Statement<'a>>, ctx: &Ctx<'a>) {
        if self.cache_vars_to_hoist.is_empty() {
            return;
        }
        let mut new_decls = ctx.ast.vec();
        for (name, kind) in self.cache_vars_to_hoist.drain(..) {
            let id = BindingIdentifier {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                name: ctx.ast.ident(&name),
                symbol_id: Cell::default(),
            };
            let init = Self::make_init_expr(kind, ctx);
            let declarator = VariableDeclarator {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                kind: VariableDeclarationKind::Var,
                id: BindingPattern::BindingIdentifier(ctx.ast.alloc(id)),
                type_annotation: None,
                init: Some(init),
                definite: false,
            };
            let mut declarations = ctx.ast.vec();
            declarations.push(declarator);
            let decl = Statement::VariableDeclaration(ctx.ast.alloc(VariableDeclaration {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                kind: VariableDeclarationKind::Var,
                declarations,
                declare: false,
            }));
            new_decls.push(decl);
        }
        for existing in statements.drain(..) {
            new_decls.push(existing);
        }
        *statements = new_decls;
    }
}

impl Default for SelfInitAccessorFlattener {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for SelfInitAccessorFlattener {
    fn exit_function(&mut self, func: &mut Function<'a>, ctx: &mut Ctx<'a>) {
        if let Some((cache_var, kind)) = Self::try_rewrite_function(func, ctx) {
            if self.rewritten < 10 {
                eprintln!(
                    "[AST/self-init] flattened accessor {} (init={kind:?}, cache={cache_var})",
                    func.id.as_ref().map(|id| id.name.as_str()).unwrap_or("?"),
                );
            }
            self.rewritten += 1;
            self.cache_vars_to_hoist.push((cache_var, kind));
        }
    }

    fn exit_program(&mut self, program: &mut Program<'a>, ctx: &mut Ctx<'a>) {
        self.hoist_cache_vars(&mut program.body, ctx);
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
        let mut pass = SelfInitAccessorFlattener::new();
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);
        traverse_mut_with_ctx(&mut pass, &mut program, &mut ctx);
        Codegen::new().build(&program).code
    }

    #[test]
    fn flattens_new_object_pattern() {
        let out = run("function hx() { var x = new Object(); hx = function() { return x; }; return x; }");
        assert!(out.contains("var __hx_cache = new Object"), "got: {out}");
        assert!(out.contains("return __hx_cache"), "got: {out}");
        assert!(!out.contains("hx = function"), "self-reassign must be gone: {out}");
    }

    #[test]
    fn flattens_empty_array_pattern() {
        let out = run("function arr() { var a = []; arr = function() { return a; }; return a; }");
        assert!(out.contains("var __arr_cache = []"));
        assert!(out.contains("return __arr_cache"));
    }

    #[test]
    fn flattens_array_entries_pattern() {
        let out = run("function it() { var i = [].entries(); it = function() { return i; }; return i; }");
        assert!(out.contains("var __it_cache = [].entries()"));
    }

    #[test]
    fn leaves_non_pattern_alone() {
        let out = run("function f() { return 1; }");
        assert!(out.contains("return 1"), "got: {out}");
        assert!(!out.contains("cache"), "got: {out}");
    }

    #[test]
    fn leaves_non_whitelisted_init_alone() {
        let out = run("function g() { var v = sideEffect(); g = function() { return v; }; return v; }");
        assert!(out.contains("sideEffect"), "must preserve side-effect init: {out}");
        assert!(out.contains("g = function"), "must preserve reassignment: {out}");
    }
}
