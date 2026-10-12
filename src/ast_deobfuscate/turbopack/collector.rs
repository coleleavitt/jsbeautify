use oxc_ast::ast::{
    Argument, ArrayExpression, AssignmentExpression, AssignmentOperator, AssignmentTarget, BindingPattern,
    CallExpression, Expression, Function, SimpleAssignmentTarget, Statement, VariableDeclarator, WithStatement,
};
use oxc_syntax::identifier::is_identifier_name;
use oxc_syntax::keyword::is_reserved_keyword_or_global_object;
use oxc_syntax::scope::ScopeId;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::FxHashSet;

use crate::ast_deobfuscate::state::DeobfuscateState;
use crate::ast_deobfuscate::webpack::shapes::{getter_target, resolve, resolve_expr, simple_params};

use super::shapes::{EsmBinding, Payload, esm_bindings, factory_params, registration};

type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

const FACTORY_PARAMS: [&str; 3] = ["__turbopack_context__", "module", "exports"];
const ASYNC_BODY_PARAMS: [&str; 2] = ["__turbopack_handle_async_dependencies__", "__turbopack_async_result__"];

/// Runtime function names by context member, from
/// `turbopack-ecmascript-runtime` (`contextPrototype.<member> = <name>`).
const RUNTIME_MEMBERS: [(&str, &str); 28] = [
    ("A", "asyncLoader"),
    ("C", "clearChunkCache"),
    ("F", "resolveFileUrl"),
    ("L", "loadChunkByUrl"),
    ("M", "moduleFactories"),
    ("P", "resolveAbsolutePath"),
    ("R", "resolvePathFromModule"),
    ("S", "esmReexport"),
    ("U", "relativeURL"),
    ("X", "ASSET_SUFFIX"),
    ("a", "asyncModule"),
    ("b", "RUNTIME_CHUNK_BASE_PATH"),
    ("c", "moduleCache"),
    ("f", "moduleContext"),
    ("h", "getChunkRelativeUrl"),
    ("i", "esmImport"),
    ("j", "dynamicExport"),
    ("l", "loadChunk"),
    ("n", "exportNamespace"),
    ("q", "exportUrl"),
    ("r", "commonJsRequire"),
    ("s", "esmExport"),
    ("t", "runtimeRequire"),
    ("v", "exportValue"),
    ("w", "RUNTIME_ROOT"),
    ("x", "externalRequire"),
    ("y", "externalImport"),
    ("z", "requireStub"),
];

/// Collects the desired name of every Turbopack binding, in priority order.
#[derive(Default)]
pub(super) struct NameCollector {
    /// `__turbopack_context__` parameters of the registered factories.
    contexts: FxHashSet<SymbolId>,
    /// The runtime's `Context` constructors and their prototype aliases.
    constructors: FxHashSet<SymbolId>,
    prototypes: FxHashSet<SymbolId>,
    plan: Vec<(SymbolId, String)>,
    planned: FxHashSet<SymbolId>,
    with_scopes: Vec<ScopeId>,
    registrations: usize,
    factories: usize,
}

impl NameCollector {
    /// The rename plan, plus the scopes containing a `with` statement.
    pub(super) fn into_plan(self) -> (Vec<(SymbolId, String)>, Vec<ScopeId>) {
        (self.plan, self.with_scopes)
    }

    pub(super) const fn registrations(&self) -> usize {
        self.registrations
    }

    pub(super) const fn factories(&self) -> usize {
        self.factories
    }

    pub(super) fn has_runtime(&self) -> bool {
        !self.constructors.is_empty()
    }

    fn plan(&mut self, symbol: SymbolId, name: &str) {
        if self.planned.insert(symbol) {
            self.plan.push((symbol, name.to_string()));
        }
    }

    fn plan_params(&mut self, params: Option<Vec<SymbolId>>, names: &[&str]) -> Option<SymbolId> {
        let params = params.filter(|params| params.len() <= names.len())?;
        for (symbol, name) in params.iter().zip(names) {
            self.plan(*symbol, name);
        }
        params.first().copied()
    }

    fn context_member<'b>(&self, callee: &'b Expression<'_>, ctx: &Ctx<'_>) -> Option<&'b str> {
        let Expression::StaticMemberExpression(member) = callee.without_parentheses() else {
            return None;
        };
        resolve_expr(&member.object, ctx)
            .is_some_and(|symbol| self.contexts.contains(&symbol))
            .then_some(member.property.name.as_str())
    }

    fn plan_exports(&mut self, list: &ArrayExpression<'_>, ctx: &Ctx<'_>) {
        for (name, binding) in esm_bindings(list) {
            let local = match binding {
                EsmBinding::Value(value) => resolve_expr(value, ctx),
                EsmBinding::Getter(getter) => getter_target(getter).and_then(|id| resolve(id, ctx)),
            };
            if is_recoverable_export_name(name)
                && let Some(symbol) = local
            {
                self.plan(symbol, name);
            }
        }
    }

    /// `x = __turbopack_context__.i(123)` / `.r(123)` → `module_123`.
    fn import_name(&self, init: &Expression<'_>, ctx: &Ctx<'_>) -> Option<String> {
        let Expression::CallExpression(call) = init.without_parentheses() else {
            return None;
        };
        let [Argument::NumericLiteral(id)] = call.arguments.as_slice() else {
            return None;
        };
        (matches!(self.context_member(&call.callee, ctx), Some("i" | "r"))
            && id.value >= 0.0
            && id.value.fract() == 0.0)
            .then(|| format!("module_{}", id.value))
    }

    fn is_prototype(&self, expr: &Expression<'_>, ctx: &Ctx<'_>) -> bool {
        match expr.without_parentheses() {
            Expression::Identifier(id) => resolve(id, ctx).is_some_and(|symbol| self.prototypes.contains(&symbol)),
            Expression::StaticMemberExpression(member) => {
                member.property.name == "prototype"
                    && resolve_expr(&member.object, ctx).is_some_and(|symbol| self.constructors.contains(&symbol))
            }
            _ => false,
        }
    }
}

/// Export names worth giving to a local: legal binding names other than `default`.
fn is_recoverable_export_name(name: &str) -> bool {
    name != "default"
        && is_identifier_name(name)
        && !is_reserved_keyword_or_global_object(name)
        && !matches!(name, "arguments" | "eval")
}

/// `this.<prop> = <param>` resolving to `param`.
fn this_assignment<'b>(expr: &'b Expression<'_>, ctx: &Ctx<'_>) -> Option<(&'b str, SymbolId)> {
    let Expression::AssignmentExpression(assign) = expr else {
        return None;
    };
    let AssignmentTarget::StaticMemberExpression(member) = &assign.left else {
        return None;
    };
    if assign.operator != AssignmentOperator::Assign || !matches!(member.object, Expression::ThisExpression(_)) {
        return None;
    }
    Some((member.property.name.as_str(), resolve_expr(&assign.right, ctx)?))
}

/// `function Context(module, exports) { this.m = module; this.e = exports }`.
fn is_context_constructor(func: &Function<'_>, ctx: &Ctx<'_>) -> bool {
    let params = simple_params(&func.params);
    let (Some(body), Some([module, exports])) = (func.body.as_ref(), params.as_deref()) else {
        return false;
    };
    let mut assignments = Vec::new();
    for statement in &body.statements {
        let Statement::ExpressionStatement(stmt) = statement else {
            return false;
        };
        match &stmt.expression {
            Expression::SequenceExpression(seq) => assignments.extend(seq.expressions.iter()),
            expr => assignments.push(expr),
        }
    }
    let assigned: Option<Vec<_>> = assignments.into_iter().map(|e| this_assignment(e, ctx)).collect();
    matches!(assigned.as_deref(), Some([("m", m), ("e", e)]) if m == module && e == exports)
}

impl<'a> Traverse<'a, DeobfuscateState> for NameCollector {
    fn enter_function(&mut self, func: &mut Function<'a>, ctx: &mut Ctx<'a>) {
        if let Some(symbol) = func.id.as_ref().and_then(|id| id.symbol_id.get())
            && is_context_constructor(func, ctx)
        {
            self.constructors.insert(symbol);
            self.plan(symbol, "Context");
        }
    }

    fn enter_call_expression(&mut self, call: &mut CallExpression<'a>, ctx: &mut Ctx<'a>) {
        if let Some(registration) = registration(call) {
            self.registrations += 1;
            if let Payload::Factories(groups) = registration.payload {
                for group in groups {
                    self.factories += 1;
                    let params = factory_params(group.factory).and_then(simple_params);
                    if let Some(context) = self.plan_params(params, &FACTORY_PARAMS) {
                        self.contexts.insert(context);
                    }
                }
            }
            return;
        }

        match self.context_member(&call.callee, ctx) {
            Some("s") => {
                if let Some(Argument::ArrayExpression(list)) = call.arguments.first() {
                    self.plan_exports(list, ctx);
                }
            }
            Some("a") => {
                let body = call.arguments.first().and_then(Argument::as_expression);
                let params = match body.map(Expression::without_parentheses) {
                    Some(Expression::ArrowFunctionExpression(arrow)) if arrow.r#async => simple_params(&arrow.params),
                    Some(Expression::FunctionExpression(func)) if func.r#async && !func.generator => {
                        simple_params(&func.params)
                    }
                    _ => None,
                };
                self.plan_params(params, &ASYNC_BODY_PARAMS);
            }
            _ => {}
        }
    }

    fn enter_variable_declarator(&mut self, decl: &mut VariableDeclarator<'a>, ctx: &mut Ctx<'a>) {
        let BindingPattern::BindingIdentifier(id) = &decl.id else {
            return;
        };
        let (Some(symbol), Some(init)) = (id.symbol_id.get(), decl.init.as_ref()) else {
            return;
        };
        if let Some(name) = self.import_name(init, ctx) {
            self.plan(symbol, &name);
        } else if !self.constructors.is_empty() && self.is_prototype(init, ctx) {
            self.prototypes.insert(symbol);
            self.plan(symbol, "contextPrototype");
        }
    }

    /// `contextPrototype.i = esmImport` in the runtime chunk.
    fn enter_assignment_expression(&mut self, assign: &mut AssignmentExpression<'a>, ctx: &mut Ctx<'a>) {
        if self.constructors.is_empty() || assign.operator != AssignmentOperator::Assign {
            return;
        }
        let Some(SimpleAssignmentTarget::StaticMemberExpression(member)) = assign.left.as_simple_assignment_target()
        else {
            return;
        };
        if !self.is_prototype(&member.object, ctx) {
            return;
        }
        let member_name = member.property.name.as_str();
        if let Some((_, name)) = RUNTIME_MEMBERS.iter().find(|(m, _)| *m == member_name)
            && let Some(symbol) = resolve_expr(&assign.right, ctx)
        {
            self.plan(symbol, name);
        }
    }

    fn enter_with_statement(&mut self, _stmt: &mut WithStatement<'a>, ctx: &mut Ctx<'a>) {
        self.with_scopes.push(ctx.current_scope_id());
    }
}
