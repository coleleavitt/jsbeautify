use oxc_ast::ast::{BindingPattern, Expression, Function, Program, Statement, VariableDeclarator};
use oxc_traverse::Traverse;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;
use super::ast_utils::{decl_name, unwrap_cjs_iife};
use super::matchers::{
    is_commonjs_helper, is_esm_helper, is_export_helper, is_name_helper_arrow, is_tocommonjs_helper, is_toesm_helper,
};
use super::reexport::{is_copyprops_helper, is_reexport_helper};
use super::types::{BunHelper, BunHelperMap};

/// Collector phase: walks program (incl. CJS-wrapper IIFE body) and identifies helpers.
pub struct BunRuntimeCollector {
    map: BunHelperMap,
}

impl BunRuntimeCollector {
    #[must_use]
    pub fn new() -> Self {
        Self {
            map: BunHelperMap::default(),
        }
    }

    #[must_use]
    pub fn into_map(self) -> BunHelperMap {
        self.map
    }

    #[must_use]
    pub const fn map(&self) -> &BunHelperMap {
        &self.map
    }

    /// Get the mangled name of a helper if detected.
    #[must_use]
    pub fn lookup_mangled(&self, helper: BunHelper) -> Option<&str> {
        self.map.iter().find_map(|(k, v)| (*v == helper).then_some(k.as_str()))
    }

    fn record(&mut self, mangled: &str, helper: BunHelper) {
        // Don't overwrite — first detection wins (helpers defined exactly once).
        if !self.map.contains_key(mangled) {
            eprintln!("[BUN] {mangled} = {} ({:?})", helper.canonical(), helper);
            self.map.insert(mangled.to_string(), helper);
        }
    }

    /// Walk a list of statements and detect helpers. Used both for top-level
    /// program body and for the body of a Bun CJS-wrapper IIFE.
    fn scan_statements(&mut self, stmts: &[Statement<'_>]) {
        for stmt in stmts {
            match stmt {
                Statement::VariableDeclaration(var_decl) => {
                    for decl in &var_decl.declarations {
                        self.try_object_alias(decl);
                        self.try_object_proto_alias(decl);
                        self.try_destructured_object_alias(decl);
                        self.try_arrow_helper(decl);
                    }
                }
                Statement::FunctionDeclaration(func) => {
                    self.try_function_helper(func);
                }
                Statement::ExpressionStatement(es) => {
                    // Look inside (function(...) { ... })(...) — Bun's CJS wrapper.
                    if let Some(body_stmts) = unwrap_cjs_iife(&es.expression) {
                        eprintln!(
                            "[BUN] descending into CJS wrapper IIFE ({} statements)",
                            body_stmts.len()
                        );
                        self.scan_statements(body_stmts);
                    }
                }
                _ => {}
            }
        }
    }

    /// `var X = Object.create;` → `__create`
    fn try_object_alias(&mut self, decl: &VariableDeclarator<'_>) {
        let Some(name) = decl_name(decl) else { return };
        let Some(init) = &decl.init else { return };
        let Expression::StaticMemberExpression(m) = init else {
            return;
        };
        let Expression::Identifier(obj) = &m.object else { return };
        if obj.name.as_str() != "Object" {
            return;
        }
        let helper = match m.property.name.as_str() {
            "create" => BunHelper::Create,
            "getPrototypeOf" => BunHelper::GetProtoOf,
            "defineProperty" => BunHelper::DefProp,
            "getOwnPropertyNames" => BunHelper::GetOwnPropNames,
            "getOwnPropertyDescriptor" => BunHelper::GetOwnPropDesc,
            _ => return,
        };
        self.record(name, helper);
    }

    /// `var X = Object.prototype.hasOwnProperty;` → `__hasOwnProp`
    fn try_object_proto_alias(&mut self, decl: &VariableDeclarator<'_>) {
        let Some(name) = decl_name(decl) else { return };
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
            _ => return,
        };
        self.record(name, helper);
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
            let helper = match key_name.as_ref() {
                "create" => BunHelper::Create,
                "getPrototypeOf" => BunHelper::GetProtoOf,
                "defineProperty" => BunHelper::DefProp,
                "getOwnPropertyNames" => BunHelper::GetOwnPropNames,
                "getOwnPropertyDescriptor" => BunHelper::GetOwnPropDesc,
                _ => continue,
            };
            self.record(val_ident.name.as_str(), helper);
        }
    }

    /// Match arrow-function helpers: `__esm`, `__commonJS`, `__toESM`, `__toCommonJS`, `__export`.
    fn try_arrow_helper(&mut self, decl: &VariableDeclarator<'_>) {
        let Some(name) = decl_name(decl) else { return };
        let Some(init) = &decl.init else { return };
        let Expression::ArrowFunctionExpression(arrow) = init else {
            return;
        };

        if is_esm_helper(arrow) {
            self.record(name, BunHelper::Esm);
        } else if is_commonjs_helper(arrow) {
            self.record(name, BunHelper::CommonJs);
        } else if is_export_helper(arrow) {
            self.record(name, BunHelper::Export);
        } else if is_toesm_helper(arrow) {
            self.record(name, BunHelper::ToEsm);
        } else if is_tocommonjs_helper(arrow) {
            self.record(name, BunHelper::ToCommonJs);
        } else if is_reexport_helper(arrow, &self.map) {
            self.record(name, BunHelper::ReExport);
        } else if is_copyprops_helper(arrow, &self.map) {
            self.record(name, BunHelper::CopyProps);
        } else if is_name_helper_arrow(arrow) {
            self.record(name, BunHelper::Name);
        }
    }

    /// `function JG$(H) { return this[H]; }` → `__accessProp`
    fn try_function_helper(&mut self, func: &Function<'_>) {
        let Some(id) = &func.id else { return };
        let name = id.name.as_str();
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
            self.record(name, BunHelper::AccessProp);
        }
    }
}

impl Default for BunRuntimeCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for BunRuntimeCollector {
    fn enter_program(&mut self, program: &mut Program<'a>, _ctx: &mut Ctx<'a>) {
        self.scan_statements(&program.body);
    }
}
