use oxc_ast::ast::{
    Argument, ArrayExpressionElement, BindingPattern, CallExpression, Expression, Program, VariableDeclarator,
    WithStatement,
};
use oxc_syntax::identifier::is_identifier_name;
use oxc_syntax::keyword::is_reserved_keyword_or_global_object;
use oxc_syntax::scope::ScopeId;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::Traverse;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::default_export::{DefaultExport, default_export_names};
use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Runtime;
use super::shapes::{
    Ctx, chunk_payload, function_params, getter_target, module_factories, resolve, resolve_expr, simple_params,
};

const FACTORY_PARAMS: [&str; 3] = ["module", "exports", "__webpack_require__"];

/// Collects the desired name of every webpack binding, in priority order.
pub(super) struct NameCollector {
    modules: FxHashSet<SymbolId>,
    requires: FxHashSet<SymbolId>,
    plan: Vec<(SymbolId, String)>,
    planned: FxHashSet<SymbolId>,
    imports: FxHashMap<SymbolId, String>,
    /// Module id of each factory's `exports` parameter.
    module_ids: FxHashMap<SymbolId, String>,
    /// Locals exported as `default`, named after every other binding.
    defaults: Vec<DefaultExport>,
    with_scopes: Vec<ScopeId>,
    /// Module factories seen in runtime module maps and chunk payloads.
    module_count: usize,
}

impl NameCollector {
    pub(super) fn new(runtimes: &[Runtime]) -> Self {
        let mut collector = Self {
            modules: runtimes.iter().map(|r| r.modules).collect(),
            requires: runtimes.iter().map(|r| r.require).collect(),
            plan: Vec::new(),
            planned: FxHashSet::default(),
            imports: FxHashMap::default(),
            module_ids: FxHashMap::default(),
            defaults: Vec::new(),
            with_scopes: Vec::new(),
            module_count: 0,
        };
        for runtime in runtimes {
            collector.plan(runtime.require, "__webpack_require__");
            collector.plan(runtime.module_id, "moduleId");
            collector.plan(runtime.modules, "__webpack_modules__");
            if let Some(cache) = runtime.cache {
                collector.plan(cache, "__webpack_module_cache__");
            }
        }
        collector
    }

    pub(super) const fn module_count(&self) -> usize {
        self.module_count
    }

    /// The rename plan, plus the scopes containing a `with` statement.
    pub(super) fn into_plan(self) -> (Vec<(SymbolId, String)>, Vec<ScopeId>) {
        (self.plan, self.with_scopes)
    }

    fn plan(&mut self, symbol: SymbolId, name: &str) {
        if self.planned.insert(symbol) {
            self.plan.push((symbol, name.to_string()));
        }
    }

    fn name_factories(&mut self, modules: &Expression<'_>) {
        let factories = module_factories(modules).unwrap_or_default();
        self.module_count += factories.len();
        for (id, params) in factories {
            let Some(symbols) = simple_params(params) else { continue };
            if symbols.len() > FACTORY_PARAMS.len() {
                continue;
            }
            if let Some(&exports) = symbols.get(1) {
                self.module_ids.insert(exports, id);
            }
            for (symbol, name) in symbols.iter().zip(FACTORY_PARAMS) {
                self.plan(*symbol, name);
            }
            if let Some(&require) = symbols.get(2) {
                self.requires.insert(require);
            }
        }
    }

    fn is_require(&self, expr: &Expression<'_>, ctx: &Ctx<'_>) -> bool {
        resolve_expr(expr, ctx).is_some_and(|symbol| self.requires.contains(&symbol))
    }

    fn plan_export(&mut self, name: &str, local: Option<SymbolId>, module: Option<&String>) {
        let Some(symbol) = local else { return };
        if name == "default" {
            self.defaults.push(DefaultExport {
                local: symbol,
                module: module.cloned(),
            });
        } else if is_recoverable_export_name(name) {
            self.plan(symbol, name);
        }
    }

    fn plan_getter_export(&mut self, name: &str, getter: &Expression<'_>, module: Option<&String>, ctx: &Ctx<'_>) {
        let local = getter_target(getter).and_then(|id| resolve(id, ctx));
        self.plan_export(name, local, module);
    }

    /// `R.d(exports, {name: getter})`, `R.d(exports, ["name", getter, "name2", 0, value])`
    /// and webpack 4's `R.d(exports, "name", getter)`.
    fn collect_exports(&mut self, args: &[Argument<'_>], ctx: &Ctx<'_>) {
        let module = args
            .first()
            .and_then(Argument::as_expression)
            .and_then(|exports| resolve_expr(exports, ctx))
            .and_then(|exports| self.module_ids.get(&exports).cloned());
        let module = module.as_ref();
        match args {
            [_, Argument::ObjectExpression(obj)] => {
                for prop in &obj.properties {
                    if let oxc_ast::ast::ObjectPropertyKind::ObjectProperty(p) = prop
                        && !p.computed
                        && let Some(name) = p.key.static_name()
                    {
                        self.plan_getter_export(&name, &p.value, module, ctx);
                    }
                }
            }
            [_, Argument::ArrayExpression(arr)] => {
                let mut elements = arr.elements.iter();
                while let Some(ArrayExpressionElement::StringLiteral(name)) = elements.next() {
                    match elements.next() {
                        Some(ArrayExpressionElement::NumericLiteral(flag)) if flag.value == 0.0 => {
                            let Some(value) = elements.next().and_then(ArrayExpressionElement::as_expression) else {
                                break;
                            };
                            self.plan_export(&name.value, resolve_expr(value, ctx), module);
                        }
                        Some(getter) => {
                            let Some(getter) = getter.as_expression() else { break };
                            self.plan_getter_export(&name.value, getter, module, ctx);
                        }
                        None => break,
                    }
                }
            }
            [_, Argument::StringLiteral(name), getter] => {
                if let Some(getter) = getter.as_expression() {
                    self.plan_getter_export(&name.value, getter, module, ctx);
                }
            }
            _ => {}
        }
    }

    /// `x = R(123)` → `module_123`; `y = R.n(x)` → `module_123_default`.
    fn import_name(&self, init: &Expression<'_>, ctx: &Ctx<'_>) -> Option<String> {
        let Expression::CallExpression(call) = init.without_parentheses() else {
            return None;
        };
        let [arg] = call.arguments.as_slice() else {
            return None;
        };
        if self.is_require(&call.callee, ctx) {
            let Argument::NumericLiteral(id) = arg else { return None };
            return (id.value >= 0.0 && id.value.fract() == 0.0).then(|| format!("module_{}", id.value));
        }
        let Expression::StaticMemberExpression(member) = &call.callee else {
            return None;
        };
        if member.property.name != "n" || !self.is_require(&member.object, ctx) {
            return None;
        }
        let module = resolve_expr(arg.as_expression()?, ctx)?;
        self.imports.get(&module).map(|name| format!("{name}_default"))
    }
}

/// Export names worth recovering: legal binding names that are not webpack's
/// deterministic 1–2 character mangled export names (`A`, `KV`) or `default`.
fn is_recoverable_export_name(name: &str) -> bool {
    name.len() > 2
        && name != "default"
        && is_identifier_name(name)
        && !is_reserved_keyword_or_global_object(name)
        && !matches!(name, "arguments" | "eval")
}

impl<'a> Traverse<'a, DeobfuscateState> for NameCollector {
    fn exit_program(&mut self, program: &mut Program<'a>, ctx: &mut Ctx<'a>) {
        let defaults = std::mem::take(&mut self.defaults);
        for (symbol, name) in default_export_names(program, ctx.scoping(), defaults) {
            self.plan(symbol, &name);
        }
    }

    fn enter_variable_declarator(&mut self, decl: &mut VariableDeclarator<'a>, ctx: &mut Ctx<'a>) {
        let BindingPattern::BindingIdentifier(id) = &decl.id else {
            return;
        };
        let (Some(symbol), Some(init)) = (id.symbol_id.get(), decl.init.as_ref()) else {
            return;
        };
        if self.modules.contains(&symbol) {
            self.name_factories(init);
        } else if let Some(name) = self.import_name(init, ctx) {
            self.plan(symbol, &name);
            self.imports.insert(symbol, name);
        }
    }

    fn enter_call_expression(&mut self, call: &mut CallExpression<'a>, ctx: &mut Ctx<'a>) {
        if let Some((modules, chunk_runtime)) = chunk_payload(call) {
            self.name_factories(modules);
            if let Some(&require) = chunk_runtime
                .and_then(function_params)
                .and_then(simple_params)
                .as_deref()
                .and_then(<[SymbolId]>::first)
            {
                self.plan(require, "__webpack_require__");
                self.requires.insert(require);
            }
            return;
        }

        if let Some(params) = function_params(&call.callee).and_then(simple_params) {
            for (i, param) in params.iter().enumerate() {
                if self.modules.contains(param)
                    && let Some(modules) = call.arguments.get(i).and_then(Argument::as_expression)
                {
                    self.name_factories(modules);
                }
            }
            return;
        }

        if let Expression::StaticMemberExpression(member) = &call.callee
            && member.property.name == "d"
            && self.is_require(&member.object, ctx)
        {
            self.collect_exports(&call.arguments, ctx);
        }
    }

    fn enter_with_statement(&mut self, _stmt: &mut WithStatement<'a>, ctx: &mut Ctx<'a>) {
        self.with_scopes.push(ctx.current_scope_id());
    }
}
