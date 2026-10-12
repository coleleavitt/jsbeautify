use oxc_ast::ast::{BindingIdentifier, BindingPattern, Expression, Function, Program, Statement, VariableDeclarator};
use oxc_span::GetSpan;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::Traverse;
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;
use super::ast_utils::{decl_binding, unwrap_cjs_iife, unwrap_iife};
use super::lowering::{is_require_shim, lowering_helper};
use super::matchers::{
    constructs_weak_map, export_descriptor_keys, is_commonjs_helper, is_esm_helper, is_export_helper,
    is_name_helper_arrow, is_tocommonjs_helper, is_toesm_helper,
};
use super::reexport::{is_copyprops_helper, is_reexport_helper};
use super::types::{BunHelper, BunHelperMap, BundleRuntime, Bundler};

/// Collector phase: walks program (incl. CJS-wrapper IIFE body) and identifies
/// the runtime helpers esbuild and Bun share, plus the variants that tell the
/// two bundlers apart.
pub struct BunRuntimeCollector {
    map: BunHelperMap,
    symbols: FxHashMap<SymbolId, BunHelper>,
    bun_evidence: bool,
    esbuild_evidence: bool,
}

impl BunRuntimeCollector {
    #[must_use]
    pub fn new() -> Self {
        Self {
            map: BunHelperMap::default(),
            symbols: FxHashMap::default(),
            bun_evidence: false,
            esbuild_evidence: false,
        }
    }

    #[must_use]
    pub fn into_map(self) -> BunHelperMap {
        self.map
    }

    #[must_use]
    pub fn into_runtime(self) -> BundleRuntime {
        let bundler = self.bundler();
        BundleRuntime {
            helpers: self.map,
            bundler,
        }
    }

    /// The bundler identified by a helper variant only it emits: Bun's
    /// `// @bun` pragma, its `__export` with a `set` accessor, or its
    /// `WeakMap`-cached `__toESM` / `__toCommonJS`; esbuild's `__export`
    /// descriptor of exactly `{ get, enumerable }`. `None` when there is no
    /// such evidence, or evidence for both.
    #[must_use]
    pub const fn bundler(&self) -> Option<Bundler> {
        match (self.bun_evidence, self.esbuild_evidence) {
            (true, false) => Some(Bundler::Bun),
            (false, true) => Some(Bundler::Esbuild),
            _ => None,
        }
    }

    #[must_use]
    pub const fn map(&self) -> &BunHelperMap {
        &self.map
    }

    /// The detected helper bindings, keyed by resolved symbol.
    #[must_use]
    pub const fn helper_symbols(&self) -> &FxHashMap<SymbolId, BunHelper> {
        &self.symbols
    }

    /// Get the mangled name of a helper if detected.
    #[must_use]
    pub fn lookup_mangled(&self, helper: BunHelper) -> Option<&str> {
        self.map.iter().find_map(|(k, v)| (*v == helper).then_some(k.as_str()))
    }

    fn record(&mut self, binding: &BindingIdentifier<'_>, helper: BunHelper) {
        if let Some(symbol) = binding.symbol_id.get() {
            self.symbols.entry(symbol).or_insert(helper);
        }
        // Don't overwrite — first detection wins (helpers defined exactly once).
        self.map.entry(binding.name.to_string()).or_insert(helper);
    }

    /// Walk a list of statements and detect helpers. Used for the top-level
    /// program body, the body of a Bun CJS-wrapper IIFE, and the body of an
    /// esbuild `--format=iife` bundle (`(() => { … })()`, or
    /// `var name = (() => { … })()` with `--global-name`).
    fn scan_statements(&mut self, stmts: &[Statement<'_>]) {
        for stmt in stmts {
            match stmt {
                Statement::VariableDeclaration(var_decl) => {
                    for decl in &var_decl.declarations {
                        self.try_object_alias(decl);
                        self.try_object_proto_alias(decl);
                        self.try_destructured_object_alias(decl);
                        self.try_arrow_helper(decl);
                        if is_require_shim(decl)
                            && let Some(binding) = decl_binding(decl)
                        {
                            self.record(binding, BunHelper::Require);
                        }
                    }
                    if let [decl] = var_decl.declarations.as_slice()
                        && let Some(body_stmts) = decl.init.as_ref().and_then(unwrap_iife)
                    {
                        self.scan_statements(body_stmts);
                    }
                }
                Statement::FunctionDeclaration(func) => {
                    self.try_function_helper(func);
                }
                Statement::ExpressionStatement(es) => {
                    // Look inside (function(...) { ... })(...) — Bun's CJS wrapper.
                    if let Some(body_stmts) = unwrap_cjs_iife(&es.expression).or_else(|| unwrap_iife(&es.expression)) {
                        self.scan_statements(body_stmts);
                    }
                }
                _ => {}
            }
        }
    }

    /// `var X = Object.create;` → `__create`, also `Reflect.get` and `Math.pow`.
    fn try_object_alias(&mut self, decl: &VariableDeclarator<'_>) {
        let Some(binding) = decl_binding(decl) else { return };
        let Some(init) = &decl.init else { return };
        let Expression::StaticMemberExpression(m) = init else {
            return;
        };
        let Expression::Identifier(obj) = &m.object else { return };
        let helper = match (obj.name.as_str(), m.property.name.as_str()) {
            ("Object", property) => match object_function_helper(property) {
                Some(helper) => helper,
                None => return,
            },
            ("Reflect", "get") => BunHelper::ReflectGet,
            ("Reflect", "set") => BunHelper::ReflectSet,
            ("Math", "pow") => BunHelper::Pow,
            _ => return,
        };
        self.record(binding, helper);
    }

    /// `var X = Object.prototype.hasOwnProperty;` → `__hasOwnProp`
    fn try_object_proto_alias(&mut self, decl: &VariableDeclarator<'_>) {
        let Some(binding) = decl_binding(decl) else { return };
        let Some(init) = &decl.init else { return };
        let Expression::StaticMemberExpression(outer) = init else {
            return;
        };
        let Expression::StaticMemberExpression(inner) = &outer.object else {
            return;
        };
        let Expression::Identifier(obj) = &inner.object else {
            return;
        };
        if obj.name.as_str() != "Object" || inner.property.name.as_str() != "prototype" {
            return;
        }
        let helper = match outer.property.name.as_str() {
            "hasOwnProperty" => BunHelper::HasOwnProp,
            "propertyIsEnumerable" => BunHelper::PropIsEnum,
            _ => return,
        };
        self.record(binding, helper);
    }

    /// `var { defineProperty: fjH, getOwnPropertyNames: UP8, … } = Object;`
    fn try_destructured_object_alias(&mut self, decl: &VariableDeclarator<'_>) {
        let BindingPattern::ObjectPattern(obj_pat) = &decl.id else {
            return;
        };
        let Some(init) = &decl.init else { return };
        let Expression::Identifier(obj) = init else { return };
        if obj.name.as_str() != "Object" {
            return;
        }
        for prop in &obj_pat.properties {
            let Some(key_name) = prop.key.name() else { continue };
            let BindingPattern::BindingIdentifier(val_ident) = &prop.value else {
                continue;
            };
            if let Some(helper) = object_function_helper(key_name.as_ref()) {
                self.record(val_ident, helper);
            }
        }
    }

    /// Match arrow-function helpers: `__esm`, `__commonJS`, `__toESM`, `__toCommonJS`, `__export`,
    /// `__reExport`, `__copyProps`, `__name`, and the [`lowering_helper`]s.
    fn try_arrow_helper(&mut self, decl: &VariableDeclarator<'_>) {
        let Some(binding) = decl_binding(decl) else { return };
        let Some(init) = &decl.init else { return };
        let Expression::ArrowFunctionExpression(arrow) = init else {
            return;
        };

        if is_esm_helper(arrow) {
            self.record(binding, BunHelper::Esm);
        } else if is_commonjs_helper(arrow) {
            self.record(binding, BunHelper::CommonJs);
        } else if is_export_helper(arrow) {
            let mut keys = export_descriptor_keys(arrow);
            keys.sort_unstable();
            if keys.iter().any(|key| key == "set") {
                self.bun_evidence = true;
            } else if keys == ["enumerable", "get"] {
                self.esbuild_evidence = true;
            }
            self.record(binding, BunHelper::Export);
        } else if is_toesm_helper(arrow, &self.map) {
            self.bun_evidence |= constructs_weak_map(arrow);
            self.record(binding, BunHelper::ToEsm);
        } else if is_tocommonjs_helper(arrow, &self.map) {
            self.bun_evidence |= constructs_weak_map(arrow);
            self.record(binding, BunHelper::ToCommonJs);
        } else if is_reexport_helper(arrow, &self.map) {
            self.record(binding, BunHelper::ReExport);
        } else if is_copyprops_helper(arrow, &self.map) {
            self.record(binding, BunHelper::CopyProps);
        } else if is_name_helper_arrow(arrow, &self.map) {
            self.record(binding, BunHelper::Name);
        } else if let Some(helper) = lowering_helper(arrow, &self.map) {
            self.record(binding, helper);
        }
    }

    /// `function JG$(H) { return this[H]; }` → `__accessProp`
    fn try_function_helper(&mut self, func: &Function<'_>) {
        let Some(id) = &func.id else { return };
        let Some(body) = &func.body else { return };
        if func.params.items.len() != 1 || body.statements.len() != 1 {
            return;
        }
        let Statement::ReturnStatement(ret) = &body.statements[0] else {
            return;
        };
        let Some(arg) = &ret.argument else { return };
        let Expression::ComputedMemberExpression(member) = arg else {
            return;
        };
        let Expression::ThisExpression(_) = &member.object else {
            return;
        };
        // Property must reference the single param.
        let BindingPattern::BindingIdentifier(param_ident) = &func.params.items[0].pattern else {
            return;
        };
        let Expression::Identifier(prop_id) = &member.expression else {
            return;
        };
        if prop_id.name.as_str() == param_ident.name.as_str() {
            self.record(id, BunHelper::AccessProp);
        }
    }
}

/// The helper aliasing `Object.<property>`.
fn object_function_helper(property: &str) -> Option<BunHelper> {
    Some(match property {
        "create" => BunHelper::Create,
        "freeze" => BunHelper::Freeze,
        "getPrototypeOf" => BunHelper::GetProtoOf,
        "defineProperty" => BunHelper::DefProp,
        "defineProperties" => BunHelper::DefProps,
        "getOwnPropertyNames" => BunHelper::GetOwnPropNames,
        "getOwnPropertyDescriptor" => BunHelper::GetOwnPropDesc,
        "getOwnPropertyDescriptors" => BunHelper::GetOwnPropDescs,
        "getOwnPropertySymbols" => BunHelper::GetOwnPropSymbols,
        _ => return None,
    })
}

impl Default for BunRuntimeCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for BunRuntimeCollector {
    fn enter_program(&mut self, program: &mut Program<'a>, _ctx: &mut Ctx<'a>) {
        let body_start = program.body.first().map_or(u32::MAX, |stmt| stmt.span().start);
        self.bun_evidence |= program
            .comments
            .iter()
            .take_while(|comment| comment.span.end <= body_start)
            .any(|comment| {
                comment
                    .content_span()
                    .source_text(program.source_text)
                    .trim_start()
                    .starts_with("@bun")
            });
        self.scan_statements(&program.body);
    }
}
