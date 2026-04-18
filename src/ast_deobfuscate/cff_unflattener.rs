//! CFF (Control-Flow-Flattening) unflattener.
//!
//! Consumes a [`DispatcherMap`] (from [`super::dispatcher_detector`]) and inlines
//! dispatcher call sites by replacing them with the cloned case body.
//!
//! Statement-level calls emit bare statements (or a block with `var` binding when
//! the body references the dispatcher's args parameter). Expression-level calls
//! fall back to a zero-param IIFE.

use oxc_allocator::{CloneIn, Vec as OxcVec};
use oxc_ast::ast::{
    BindingIdentifier, BindingPattern, BlockStatement, CallExpression, Expression, FormalParameterKind,
    FormalParameters, Function, FunctionBody, FunctionType, Program, Statement, VariableDeclaration,
    VariableDeclarationKind, VariableDeclarator,
};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use oxc_traverse::{Ancestor, Traverse, TraverseCtx};
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
            scan_expr(&es.expression, dispatchers, bodies, alloc);
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
        Statement::ForInStatement(f) => scan_stmt(&f.body, dispatchers, bodies, alloc),
        Statement::ForOfStatement(f) => scan_stmt(&f.body, dispatchers, bodies, alloc),
        Statement::WhileStatement(w) => scan_stmt(&w.body, dispatchers, bodies, alloc),
        Statement::DoWhileStatement(d) => scan_stmt(&d.body, dispatchers, bodies, alloc),
        Statement::LabeledStatement(l) => scan_stmt(&l.body, dispatchers, bodies, alloc),
        Statement::SwitchStatement(s) => {
            for case in &s.cases {
                scan_stmts(&case.consequent, dispatchers, bodies, alloc);
            }
        }
        _ => {}
    }
}

fn scan_expr<'a>(
    expr: &Expression<'a>,
    dispatchers: &DispatcherMap,
    bodies: &mut CaseBodyMap<'a>,
    alloc: &'a oxc_allocator::Allocator,
) {
    match expr {
        Expression::FunctionExpression(func) => {
            if let Some(body) = &func.body {
                scan_stmts(&body.statements, dispatchers, bodies, alloc);
            }
        }
        Expression::ArrowFunctionExpression(func) => {
            scan_stmts(&func.body.statements, dispatchers, bodies, alloc);
        }
        Expression::AssignmentExpression(assign) => {
            scan_expr(&assign.right, dispatchers, bodies, alloc);
        }
        Expression::CallExpression(call) => {
            scan_expr(&call.callee, dispatchers, bodies, alloc);
            for arg in &call.arguments {
                if let Some(e) = arg.as_expression() {
                    scan_expr(e, dispatchers, bodies, alloc);
                }
            }
        }
        Expression::NewExpression(ne) => {
            scan_expr(&ne.callee, dispatchers, bodies, alloc);
            for arg in &ne.arguments {
                if let Some(e) = arg.as_expression() {
                    scan_expr(e, dispatchers, bodies, alloc);
                }
            }
        }
        Expression::SequenceExpression(seq) => {
            for e in &seq.expressions {
                scan_expr(e, dispatchers, bodies, alloc);
            }
        }
        Expression::ParenthesizedExpression(p) => {
            scan_expr(&p.expression, dispatchers, bodies, alloc);
        }
        Expression::ConditionalExpression(c) => {
            scan_expr(&c.consequent, dispatchers, bodies, alloc);
            scan_expr(&c.alternate, dispatchers, bodies, alloc);
        }
        Expression::LogicalExpression(l) => {
            scan_expr(&l.left, dispatchers, bodies, alloc);
            scan_expr(&l.right, dispatchers, bodies, alloc);
        }
        Expression::ObjectExpression(o) => {
            for prop in &o.properties {
                if let oxc_ast::ast::ObjectPropertyKind::ObjectProperty(p) = prop {
                    scan_expr(&p.value, dispatchers, bodies, alloc);
                }
            }
        }
        Expression::ArrayExpression(a) => {
            for elem in &a.elements {
                if let Some(e) = elem.as_expression() {
                    scan_expr(e, dispatchers, bodies, alloc);
                }
            }
        }
        Expression::UnaryExpression(u) => {
            scan_expr(&u.argument, dispatchers, bodies, alloc);
        }
        Expression::BinaryExpression(b) => {
            scan_expr(&b.left, dispatchers, bodies, alloc);
            scan_expr(&b.right, dispatchers, bodies, alloc);
        }
        Expression::StaticMemberExpression(s) => {
            scan_expr(&s.object, dispatchers, bodies, alloc);
        }
        Expression::ComputedMemberExpression(c) => {
            scan_expr(&c.object, dispatchers, bodies, alloc);
            scan_expr(&c.expression, dispatchers, bodies, alloc);
        }
        Expression::TaggedTemplateExpression(t) => {
            scan_expr(&t.tag, dispatchers, bodies, alloc);
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
    // Always recurse into the function body to find nested dispatchers
    if let Some(body) = &func.body {
        scan_stmts(&body.statements, dispatchers, bodies, alloc);
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
    let switch = body.statements.iter().find_map(|s| {
        if let Statement::SwitchStatement(sw) = s {
            Some(sw)
        } else {
            None
        }
    });
    let Some(switch) = switch else { return };
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

fn body_references_param(stmts: &[Statement<'_>], param_name: &str) -> bool {
    stmts.iter().any(|s| stmt_references(s, param_name))
}

fn stmt_references(stmt: &Statement<'_>, name: &str) -> bool {
    match stmt {
        Statement::ExpressionStatement(es) => expr_references(&es.expression, name),
        Statement::ReturnStatement(r) => r.argument.as_ref().is_some_and(|e| expr_references(e, name)),
        Statement::VariableDeclaration(d) => d
            .declarations
            .iter()
            .any(|decl| decl.init.as_ref().is_some_and(|e| expr_references(e, name))),
        Statement::BlockStatement(b) => b.body.iter().any(|s| stmt_references(s, name)),
        Statement::IfStatement(i) => {
            expr_references(&i.test, name)
                || stmt_references(&i.consequent, name)
                || i.alternate.as_ref().is_some_and(|a| stmt_references(a, name))
        }
        Statement::ForStatement(f) => {
            f.test.as_ref().is_some_and(|e| expr_references(e, name))
                || f.update.as_ref().is_some_and(|e| expr_references(e, name))
                || stmt_references(&f.body, name)
        }
        Statement::ForInStatement(f) => expr_references(&f.right, name) || stmt_references(&f.body, name),
        Statement::ForOfStatement(f) => expr_references(&f.right, name) || stmt_references(&f.body, name),
        Statement::WhileStatement(w) => expr_references(&w.test, name) || stmt_references(&w.body, name),
        Statement::DoWhileStatement(d) => stmt_references(&d.body, name) || expr_references(&d.test, name),
        Statement::SwitchStatement(s) => {
            expr_references(&s.discriminant, name)
                || s.cases.iter().any(|c| {
                    c.test.as_ref().is_some_and(|e| expr_references(e, name))
                        || c.consequent.iter().any(|st| stmt_references(st, name))
                })
        }
        Statement::TryStatement(t) => {
            t.block.body.iter().any(|s| stmt_references(s, name))
                || t.handler
                    .as_ref()
                    .is_some_and(|h| h.body.body.iter().any(|s| stmt_references(s, name)))
                || t.finalizer
                    .as_ref()
                    .is_some_and(|f| f.body.iter().any(|s| stmt_references(s, name)))
        }
        Statement::ThrowStatement(t) => expr_references(&t.argument, name),
        Statement::LabeledStatement(l) => stmt_references(&l.body, name),
        _ => false,
    }
}

fn expr_references(expr: &Expression<'_>, name: &str) -> bool {
    match expr {
        Expression::Identifier(id) => id.name.as_str() == name,
        Expression::CallExpression(c) => {
            expr_references(&c.callee, name)
                || c.arguments
                    .iter()
                    .any(|a| a.as_expression().is_some_and(|e| expr_references(e, name)))
        }
        Expression::NewExpression(n) => {
            expr_references(&n.callee, name)
                || n.arguments
                    .iter()
                    .any(|a| a.as_expression().is_some_and(|e| expr_references(e, name)))
        }
        Expression::StaticMemberExpression(s) => expr_references(&s.object, name),
        Expression::ComputedMemberExpression(c) => {
            expr_references(&c.object, name) || expr_references(&c.expression, name)
        }
        Expression::AssignmentExpression(a) => expr_references(&a.right, name),
        Expression::BinaryExpression(b) => expr_references(&b.left, name) || expr_references(&b.right, name),
        Expression::LogicalExpression(l) => expr_references(&l.left, name) || expr_references(&l.right, name),
        Expression::UnaryExpression(u) => expr_references(&u.argument, name),
        Expression::UpdateExpression(u) => matches!(
            &u.argument,
            oxc_ast::ast::SimpleAssignmentTarget::AssignmentTargetIdentifier(id) if id.name.as_str() == name
        ),
        Expression::ConditionalExpression(c) => {
            expr_references(&c.test, name)
                || expr_references(&c.consequent, name)
                || expr_references(&c.alternate, name)
        }
        Expression::SequenceExpression(s) => s.expressions.iter().any(|e| expr_references(e, name)),
        Expression::ParenthesizedExpression(p) => expr_references(&p.expression, name),
        Expression::ArrayExpression(a) => a
            .elements
            .iter()
            .any(|el| el.as_expression().is_some_and(|e| expr_references(e, name))),
        Expression::ObjectExpression(o) => o.properties.iter().any(|prop| {
            if let oxc_ast::ast::ObjectPropertyKind::ObjectProperty(p) = prop {
                expr_references(&p.value, name)
            } else {
                false
            }
        }),
        Expression::FunctionExpression(f) => f
            .body
            .as_ref()
            .is_some_and(|b| b.statements.iter().any(|s| stmt_references(s, name))),
        Expression::ArrowFunctionExpression(a) => a.body.statements.iter().any(|s| stmt_references(s, name)),
        Expression::TemplateLiteral(t) => t.expressions.iter().any(|e| expr_references(e, name)),
        Expression::TaggedTemplateExpression(t) => {
            expr_references(&t.tag, name) || t.quasi.expressions.iter().any(|e| expr_references(e, name))
        }
        Expression::YieldExpression(y) => y.argument.as_ref().is_some_and(|e| expr_references(e, name)),
        Expression::AwaitExpression(a) => expr_references(&a.argument, name),
        _ => false,
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
        let uses_args = body_references_param(body_stmts.as_slice(), args_param_name);

        let mut stmts: OxcVec<'a, Statement<'a>> = ctx.ast.vec();

        if uses_args {
            let binding_id = BindingIdentifier {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                name: ctx.ast.ident(args_param_name),
                symbol_id: Cell::default(),
            };
            let declarator = VariableDeclarator {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                kind: VariableDeclarationKind::Var,
                id: BindingPattern::BindingIdentifier(ctx.ast.alloc(binding_id)),
                type_annotation: None,
                init: Some(call_site_arg.clone_in_with_semantic_ids(alloc)),
                definite: false,
            };
            let mut declarations = ctx.ast.vec();
            declarations.push(declarator);
            stmts.push(Statement::VariableDeclaration(ctx.ast.alloc(VariableDeclaration {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                kind: VariableDeclarationKind::Var,
                declarations,
                declare: false,
            })));
        }

        for s in body_stmts {
            stmts.push(s.clone_in_with_semantic_ids(alloc));
        }

        let fn_body = FunctionBody {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            directives: ctx.ast.vec(),
            statements: stmts,
        };

        let params = FormalParameters {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            kind: FormalParameterKind::FormalParameter,
            items: ctx.ast.vec(),
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

    fn make_inline_stmts(
        args_param_name: &str,
        body_stmts: &OxcVec<'a, Statement<'a>>,
        call_site_arg: &Expression<'a>,
        ctx: &Ctx<'a>,
    ) -> OxcVec<'a, Statement<'a>> {
        let alloc = ctx.ast.allocator;
        let uses_args = body_references_param(body_stmts.as_slice(), args_param_name);

        if !uses_args {
            let mut stmts: OxcVec<'a, Statement<'a>> = ctx.ast.vec();
            for s in body_stmts {
                stmts.push(s.clone_in_with_semantic_ids(alloc));
            }
            return stmts;
        }

        let mut block_body: OxcVec<'a, Statement<'a>> = ctx.ast.vec();

        let binding_id = BindingIdentifier {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            name: ctx.ast.ident(args_param_name),
            symbol_id: Cell::default(),
        };
        let declarator = VariableDeclarator {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            kind: VariableDeclarationKind::Var,
            id: BindingPattern::BindingIdentifier(ctx.ast.alloc(binding_id)),
            type_annotation: None,
            init: Some(call_site_arg.clone_in_with_semantic_ids(alloc)),
            definite: false,
        };
        let mut declarations = ctx.ast.vec();
        declarations.push(declarator);
        block_body.push(Statement::VariableDeclaration(ctx.ast.alloc(VariableDeclaration {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            kind: VariableDeclarationKind::Var,
            declarations,
            declare: false,
        })));

        for s in body_stmts {
            block_body.push(s.clone_in_with_semantic_ids(alloc));
        }

        let block = Statement::BlockStatement(ctx.ast.alloc(BlockStatement {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            body: block_body,
            scope_id: Cell::new(None),
        }));

        let mut result: OxcVec<'a, Statement<'a>> = ctx.ast.vec();
        result.push(block);
        result
    }

    fn resolve_direct<'b>(
        &'b self,
        call: &'b CallExpression<'a>,
    ) -> Option<(
        &'b str,
        &'b str,
        &'b Expression<'a>,
        &'b DispatcherMeta,
        &'b OxcVec<'a, Statement<'a>>,
    )> {
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
        let state_expr = call.arguments[0].as_expression()?;
        let Expression::Identifier(state_id) = state_expr else {
            return None;
        };
        let state_label = state_id.name.as_str();
        let key = (name.to_string(), state_label.to_string());
        if !self.case_bodies.contains_key(&key) {
            return None;
        }
        let args_expr = call.arguments[1].as_expression()?;
        let meta = self.meta.get(name)?;
        let body = self.case_bodies.get(&key)?;
        Some((name, state_label, args_expr, meta, body))
    }

    fn resolve_call_this<'b>(
        &'b self,
        call: &'b CallExpression<'a>,
    ) -> Option<(
        &'b str,
        &'b str,
        &'b Expression<'a>,
        &'b DispatcherMeta,
        &'b OxcVec<'a, Statement<'a>>,
    )> {
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
        let first = call.arguments[0].as_expression()?;
        if !matches!(first, Expression::ThisExpression(_)) {
            return None;
        }
        let state_expr = call.arguments[1].as_expression()?;
        let Expression::Identifier(state_id) = state_expr else {
            return None;
        };
        let state_label = state_id.name.as_str();
        let key = (name.to_string(), state_label.to_string());
        if !self.case_bodies.contains_key(&key) {
            return None;
        }
        let args_expr = call.arguments[2].as_expression()?;
        let meta = self.meta.get(name)?;
        let body = self.case_bodies.get(&key)?;
        Some((name, state_label, args_expr, meta, body))
    }

    fn resolve_call<'b>(
        &'b self,
        call: &'b CallExpression<'a>,
    ) -> Option<(
        &'b str,
        &'b str,
        &'b Expression<'a>,
        &'b DispatcherMeta,
        &'b OxcVec<'a, Statement<'a>>,
    )> {
        self.resolve_direct(call).or_else(|| self.resolve_call_this(call))
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for CffUnflattener<'a> {
    fn exit_statements(&mut self, stmts: &mut OxcVec<'a, Statement<'a>>, ctx: &mut Ctx<'a>) {
        let mut i = 0;
        while i < stmts.len() {
            let call = match &stmts[i] {
                Statement::ExpressionStatement(es) => {
                    if let Expression::CallExpression(c) = &es.expression {
                        Some(c.as_ref())
                    } else {
                        None
                    }
                }
                _ => None,
            };
            let Some(call) = call else {
                i += 1;
                continue;
            };
            let Some((_, _, args_expr, meta, body)) = self.resolve_call(call) else {
                i += 1;
                continue;
            };
            let args_param = meta.args_param.clone();
            let replacement = Self::make_inline_stmts(&args_param, body, args_expr, ctx);

            if self.inlined < 10 {
                eprintln!("[CFF] inlining call site (stmt-level)");
            }
            self.inlined += 1;

            let count = replacement.len();
            stmts.remove(i);
            for (j, s) in replacement.into_iter().enumerate() {
                stmts.insert(i + j, s);
            }
            i += count;
        }
    }

    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        if matches!(ctx.parent(), Ancestor::ExpressionStatementExpression(_)) {
            return;
        }
        if let Expression::CallExpression(call) = expr {
            let Some((_, _, args_expr, meta, body)) = self.resolve_call(call) else {
                return;
            };
            let args_param = meta.args_param.clone();
            let replacement = Self::make_iife(&args_param, body, args_expr, ctx);

            if self.inlined < 10 {
                eprintln!("[CFF] inlining call site (expr-level)");
            }
            self.inlined += 1;

            *expr = replacement;
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
            out.contains("var a = [5]"),
            "should have var decl for args param: {out}"
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
        assert!(
            out.contains("var a = [5]"),
            "should have var decl for args param: {out}"
        );
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

    #[test]
    fn no_args_ref_emits_bare_stmts() {
        let code = r#"
            function D(s, a) {
                switch (s) {
                    case X: { foo(); bar(); } break;
                }
            }
            D(X, [1]);
        "#;
        let (out, inlined) = run(code);
        assert_eq!(inlined, 1, "should inline: {out}");
        assert!(!out.contains("function("), "no IIFE wrapper: {out}");
        assert!(!out.contains("var a"), "no var decl for unused param: {out}");
        assert!(out.contains("foo()"), "bare stmt emitted: {out}");
        assert!(out.contains("bar()"), "bare stmt emitted: {out}");
    }

    #[test]
    fn args_ref_emits_block_with_var() {
        let code = r#"
            function D(s, a) {
                switch (s) {
                    case X: { var t = a[0]; foo(t); } break;
                }
            }
            D(X, [42]);
        "#;
        let (out, inlined) = run(code);
        assert_eq!(inlined, 1, "should inline: {out}");
        assert!(!out.contains("function(a)"), "no IIFE param: {out}");
        assert!(out.contains("var a = [42]"), "var decl for args param: {out}");
        assert!(out.contains("foo(t)"), "body preserved: {out}");
    }
}
