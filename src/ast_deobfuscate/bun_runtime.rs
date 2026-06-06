//! Bun bundler runtime detection & rename
//!
//! Bun (1.4+) wraps every single-file executable in a CJS-style IIFE:
//! ```js
//! (function(exports, require, module, __filename, __dirname) {
//!     // ... entire program here, including the helper preamble
//! })
//! ```
//!
//! Inside that IIFE, Bun emits a canonical helper preamble of mangled names
//! (extracted ground truth from the bundler binary):
//!
//! | Canonical          | Mangled in cli.2.1.132.js | Body shape                                                |
//! |--------------------|---------------------------|-----------------------------------------------------------|
//! | `__create`         | `Sd4`                     | `Object.create`                                           |
//! | `__getProtoOf`     | `Rd4` (destructured)      | `Object.getPrototypeOf`                                   |
//! | `__defProp`        | `fjH` (destructured)      | `Object.defineProperty`                                   |
//! | `__getOwnPropNames`| `UP8` (destructured)      | `Object.getOwnPropertyNames`                              |
//! | `__getOwnPropDesc` | `Cd4` (destructured)      | `Object.getOwnPropertyDescriptor`                         |
//! | `__hasOwnProp`     | `XG$`                     | `Object.prototype.hasOwnProperty`                         |
//! | `__accessProp`     | `JG$`                     | `function (key) { return this[key] }`                     |
//! | `__commonJS`       | `n`                       | `(H,$) => () => ($\|\|H(($={exports:{}}).exports,$),...)` |
//! | `__esm`            | `V`                       | `(H,$) => () => (H && ($=H(H=0)), $)`                     |
//! | `__toESM`          | `m`                       | `(mod,isNode,target) => …`                                |
//! | `__toCommonJS`     | `n6`                      | WeakMap-cached `__esModule` wrapper                       |
//! | `__export`         | `j$`                      | `for (q in $) defProp(H,q,{get:$[q],…})`                  |
//!
//! This pass:
//! 1. Detects the top-level Bun CJS wrapper IIFE.
//! 2. Inside its body, identifies each helper by AST shape.
//! 3. Renames all references to the canonical name (e.g. `V → __esm`).
//! 4. Records the map in `DeobfuscateState` for downstream passes.
//!
//! No statements are deleted; this is purely a renaming pass. Subsequent passes
//! (`bun_esm_inliner`, `bun_commonjs_inliner`) consume the canonical names.

use oxc_ast::ast::{
    Argument, BinaryOperator, BindingPattern, CallExpression, Expression, ForOfStatement, ForStatementLeft, Function,
    IdentifierReference, ObjectPropertyKind, Program, Statement, VariableDeclarator,
};

use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

/// Canonical Bun helper names — used as the rename targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BunHelper {
    Create,
    GetProtoOf,
    DefProp,
    GetOwnPropNames,
    GetOwnPropDesc,
    HasOwnProp,
    AccessProp,
    CopyProps,
    ReExport,
    CommonJs,
    Esm,
    ToEsm,
    ToCommonJs,
    Export,
    Name,
}

impl BunHelper {
    #[must_use]
    pub const fn canonical(&self) -> &'static str {
        match self {
            Self::Create => "__create",
            Self::GetProtoOf => "__getProtoOf",
            Self::DefProp => "__defProp",
            Self::GetOwnPropNames => "__getOwnPropNames",
            Self::GetOwnPropDesc => "__getOwnPropDesc",
            Self::HasOwnProp => "__hasOwnProp",
            Self::AccessProp => "__accessProp",
            Self::CopyProps => "__copyProps",
            Self::ReExport => "__reExport",
            Self::CommonJs => "__commonJS",
            Self::Esm => "__esm",
            Self::ToEsm => "__toESM",
            Self::ToCommonJs => "__toCommonJS",
            Self::Export => "__export",
            Self::Name => "__name",
        }
    }
}

/// Map from mangled name → canonical helper.
pub type BunHelperMap = FxHashMap<String, BunHelper>;

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
    pub fn map(&self) -> &BunHelperMap {
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
    fn scan_statements<'a>(&mut self, stmts: &[Statement<'a>]) {
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

// ─────────────────────────────────────────────────────────────────────────────
// Renaming pass
// ─────────────────────────────────────────────────────────────────────────────

/// Renames mangled helper identifiers to their canonical names everywhere
/// in the program, using a map from `BunRuntimeCollector`.
pub struct BunRuntimeRenamer {
    map: BunHelperMap,
    renamed_refs: usize,
    renamed_decls: usize,
}

impl BunRuntimeRenamer {
    #[must_use]
    pub fn new(map: BunHelperMap) -> Self {
        Self {
            map,
            renamed_refs: 0,
            renamed_decls: 0,
        }
    }

    #[must_use]
    pub const fn renamed_refs(&self) -> usize {
        self.renamed_refs
    }

    #[must_use]
    pub const fn renamed_decls(&self) -> usize {
        self.renamed_decls
    }

    fn canonical_for(&self, mangled: &str) -> Option<&'static str> {
        self.map.get(mangled).map(BunHelper::canonical)
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for BunRuntimeRenamer {
    fn exit_identifier_reference(&mut self, ident: &mut IdentifierReference<'a>, ctx: &mut Ctx<'a>) {
        if let Some(canon) = self.canonical_for(ident.name.as_str()) {
            ident.name = ctx.ast.ident(canon);
            self.renamed_refs += 1;
        }
    }

    fn exit_variable_declarator(&mut self, decl: &mut VariableDeclarator<'a>, ctx: &mut Ctx<'a>) {
        // Rename `var V = …` → `var __esm = …`
        if let BindingPattern::BindingIdentifier(ident) = &mut decl.id {
            if let Some(canon) = self.canonical_for(ident.name.as_str()) {
                ident.name = ctx.ast.ident(canon);
                self.renamed_decls += 1;
            }
        }
        // Rename inside `var { defineProperty: fjH, … } = Object;`
        if let BindingPattern::ObjectPattern(obj_pat) = &mut decl.id {
            for prop in obj_pat.properties.iter_mut() {
                if let BindingPattern::BindingIdentifier(b) = &mut prop.value {
                    if let Some(canon) = self.canonical_for(b.name.as_str()) {
                        b.name = ctx.ast.ident(canon);
                        self.renamed_decls += 1;
                    }
                }
            }
        }
    }

    fn exit_function(&mut self, func: &mut Function<'a>, ctx: &mut Ctx<'a>) {
        if let Some(id) = &mut func.id {
            if let Some(canon) = self.canonical_for(id.name.as_str()) {
                id.name = ctx.ast.ident(canon);
                self.renamed_decls += 1;
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AST shape matchers
// ─────────────────────────────────────────────────────────────────────────────

fn decl_name<'a, 'b>(decl: &'a VariableDeclarator<'b>) -> Option<&'a str> {
    match &decl.id {
        BindingPattern::BindingIdentifier(b) => Some(b.name.as_str()),
        _ => None,
    }
}

/// Match `(function (exports, require, module, __filename, __dirname) { ... })(...)`.
/// Match `(function (exports, require, module, __filename, __dirname) { ... })(...)`.
/// OR the Bun form: `(function(exports, require, module, __filename, __dirname) { ... })`
/// without the trailing call (the IIFE isn't invoked, just defined as an expression-statement).
/// Returns the body statements if matched.
fn unwrap_cjs_iife<'a, 'b>(expr: &'a Expression<'b>) -> Option<&'a [Statement<'b>]> {
    // Extract the FunctionExpression from various wrapping shapes.
    let func = if let Expression::CallExpression(call) = expr {
        // Case 1: Called IIFE: `(function(...){})(...)`
        match &call.callee {
            Expression::ParenthesizedExpression(p) => match &p.expression {
                Expression::FunctionExpression(f) => Some(f.as_ref()),
                _ => None,
            },
            Expression::FunctionExpression(f) => Some(f.as_ref()),
            _ => None,
        }
    } else if let Expression::ParenthesizedExpression(p) = expr {
        // Case 2: Non-called wrapper: `(function(...){})`
        match &p.expression {
            Expression::FunctionExpression(f) => Some(f.as_ref()),
            _ => None,
        }
    } else if let Expression::FunctionExpression(f) = expr {
        Some(f.as_ref())
    } else {
        None
    }?;

    let body = func.body.as_ref()?;
    // Must look like a CJS wrapper: at least 3 of {exports, require, module, __filename, __dirname}.
    let cjs_hits = func
        .params
        .items
        .iter()
        .filter_map(|p| match &p.pattern {
            BindingPattern::BindingIdentifier(b) => Some(b.name.as_str()),
            _ => None,
        })
        .filter(|n| matches!(*n, "exports" | "require" | "module" | "__filename" | "__dirname"))
        .count();
    if cjs_hits < 3 {
        return None;
    }
    Some(&body.statements)
}

/// `__esm`: `(H, $) => () => (H && ($ = H(H = 0)), $)` (plus paren variants).
fn is_esm_helper<'a>(arrow: &oxc_ast::ast::ArrowFunctionExpression<'a>) -> bool {
    if arrow.params.items.len() != 2 || !arrow.expression {
        return false;
    }
    let Some(inner) = inner_arrow_body_expr(arrow) else {
        return false;
    };
    // inner is the returned expression after paren-unwrap.
    let Expression::SequenceExpression(seq) = inner else {
        return false;
    };
    if seq.expressions.len() != 2 {
        return false;
    }
    // [0] = LogicalExpression with && operator
    let Expression::LogicalExpression(logical) = &seq.expressions[0] else {
        return false;
    };
    if !matches!(logical.operator, oxc_ast::ast::LogicalOperator::And) {
        return false;
    }
    // [1] = identifier (the result variable)
    matches!(seq.expressions[1], Expression::Identifier(_))
}

/// `__commonJS`: `(H, $) => () => ($ || H(($ = { exports: {} }).exports, $), $.exports)`
fn is_commonjs_helper<'a>(arrow: &oxc_ast::ast::ArrowFunctionExpression<'a>) -> bool {
    if arrow.params.items.len() != 2 || !arrow.expression {
        return false;
    }
    let Some(inner) = inner_arrow_body_expr(arrow) else {
        return false;
    };
    let Expression::SequenceExpression(seq) = inner else {
        return false;
    };
    if seq.expressions.len() != 2 {
        return false;
    }
    // [0] = `$ || H(...)` — LogicalExpression with || (or LogicalOr)
    let Expression::LogicalExpression(logical) = &seq.expressions[0] else {
        return false;
    };
    if !matches!(logical.operator, oxc_ast::ast::LogicalOperator::Or) {
        return false;
    }
    // [1] = `$.exports` — StaticMemberExpression with property name "exports"
    let Expression::StaticMemberExpression(member) = &seq.expressions[1] else {
        return false;
    };
    member.property.name.as_str() == "exports"
}

/// `__export`: `(target, all) => { for (var name in all) __defProp(target, name, {get: all[name], …}) }`
fn is_export_helper<'a>(arrow: &oxc_ast::ast::ArrowFunctionExpression<'a>) -> bool {
    if arrow.params.items.len() != 2 || arrow.expression {
        return false;
    }
    let Some(stmt) = arrow.body.statements.first() else {
        return false;
    };
    matches!(stmt, Statement::ForInStatement(_))
}

/// `__copyProps`: `(H, $, q) => { var K = __getOwnPropNames($); for (let _ of K) ... }`.
/// Exactly 3 params (simple identifiers, not destructuring), block body,
/// first statement is VarDecl with a CallExpression init, has for-of,
/// does NOT contain `__esModule` (which would indicate __toESM instead).
fn is_copyprops_helper<'a>(arrow: &oxc_ast::ast::ArrowFunctionExpression<'a>, helpers: &BunHelperMap) -> bool {
    if !(3..=4).contains(&arrow.params.items.len()) || arrow.expression {
        return false;
    }
    // All params must be simple identifiers (not destructured)
    for p in &arrow.params.items {
        if !matches!(p.pattern, BindingPattern::BindingIdentifier(_)) {
            return false;
        }
    }

    let Some(source_param) = param_name(&arrow.params.items[1].pattern) else {
        return false;
    };
    let Some(keys_name) = reexport_keys_binding(arrow, source_param, helpers) else {
        return false;
    };
    let has_for_of_over_keys =
        arrow.body.statements.iter().any(
            |stmt| matches!(stmt, Statement::ForOfStatement(for_of) if expression_is_ident(&for_of.right, keys_name)),
        );

    // Must NOT contain __esModule (that's __toESM)
    has_for_of_over_keys && !body_contains_esmodule_check(&arrow.body.statements)
}

/// `__reExport`: Bun's 3-param export-star helper. It skips the `"default"`
/// key and conditionally copies descriptors into a second target before
/// returning the third parameter.
fn is_reexport_helper<'a>(arrow: &oxc_ast::ast::ArrowFunctionExpression<'a>, helpers: &BunHelperMap) -> bool {
    if arrow.params.items.len() != 3 || arrow.expression {
        return false;
    }
    if !arrow
        .params
        .items
        .iter()
        .all(|p| matches!(p.pattern, BindingPattern::BindingIdentifier(_)))
    {
        return false;
    }

    let Some(target_param) = param_name(&arrow.params.items[0].pattern) else {
        return false;
    };
    let Some(source_param) = param_name(&arrow.params.items[1].pattern) else {
        return false;
    };
    let BindingPattern::BindingIdentifier(third) = &arrow.params.items[2].pattern else {
        return false;
    };
    let return_param = third.name.as_str();

    let Some(keys_name) = reexport_keys_binding(arrow, source_param, helpers) else {
        return false;
    };
    let Some(for_of) = arrow.body.statements.iter().find_map(|stmt| match stmt {
        Statement::ForOfStatement(for_of) if expression_is_ident(&for_of.right, keys_name) => Some(for_of.as_ref()),
        _ => None,
    }) else {
        return false;
    };
    let Some(loop_var) = for_of_loop_var_name(for_of) else {
        return false;
    };

    let returns_third_param = arrow
        .body
        .statements
        .iter()
        .any(|stmt| stmt_returns_ident(stmt, return_param));
    let has_default_skip = stmt_has_default_guard_for_loop_var(&for_of.body, loop_var);
    let has_descriptor_copy =
        stmt_has_reexport_define_property_copy(&for_of.body, target_param, source_param, loop_var, helpers);

    returns_third_param
        && has_default_skip
        && has_descriptor_copy
        && !body_contains_esmodule_check(&arrow.body.statements)
}

fn param_name<'a>(pattern: &'a BindingPattern<'a>) -> Option<&'a str> {
    let BindingPattern::BindingIdentifier(binding) = pattern else {
        return None;
    };
    Some(binding.name.as_str())
}

fn reexport_keys_binding<'a>(
    arrow: &'a oxc_ast::ast::ArrowFunctionExpression<'a>,
    source_param: &str,
    helpers: &BunHelperMap,
) -> Option<&'a str> {
    let Statement::VariableDeclaration(var_decl) = arrow.body.statements.first()? else {
        return None;
    };
    let decl = var_decl.declarations.first()?;
    let BindingPattern::BindingIdentifier(binding) = &decl.id else {
        return None;
    };
    let Some(Expression::CallExpression(call)) = &decl.init else {
        return None;
    };
    is_get_own_prop_names_call(call, source_param, helpers).then_some(binding.name.as_str())
}

fn is_get_own_prop_names_call(call: &CallExpression<'_>, source_param: &str, helpers: &BunHelperMap) -> bool {
    call.arguments
        .first()
        .is_some_and(|arg| argument_is_ident(arg, source_param))
        && call_callee_is_helper_or_object_property(call, helpers, BunHelper::GetOwnPropNames, "getOwnPropertyNames")
}

fn for_of_loop_var_name<'a>(for_of: &'a ForOfStatement<'a>) -> Option<&'a str> {
    match &for_of.left {
        ForStatementLeft::VariableDeclaration(var_decl) => {
            let decl = var_decl.declarations.first()?;
            param_name(&decl.id)
        }
        ForStatementLeft::AssignmentTargetIdentifier(ident) => Some(ident.name.as_str()),
        _ => None,
    }
}

fn stmt_has_default_guard_for_loop_var(stmt: &Statement<'_>, loop_var: &str) -> bool {
    match stmt {
        Statement::IfStatement(if_stmt) => {
            expr_has_default_guard_for_loop_var(&if_stmt.test, loop_var)
                || stmt_has_default_guard_for_loop_var(&if_stmt.consequent, loop_var)
                || if_stmt
                    .alternate
                    .as_ref()
                    .is_some_and(|alternate| stmt_has_default_guard_for_loop_var(alternate, loop_var))
        }
        Statement::BlockStatement(block) => block
            .body
            .iter()
            .any(|stmt| stmt_has_default_guard_for_loop_var(stmt, loop_var)),
        _ => false,
    }
}

fn expr_has_default_guard_for_loop_var(expr: &Expression<'_>, loop_var: &str) -> bool {
    match expr {
        Expression::BinaryExpression(binary)
            if matches!(
                binary.operator,
                BinaryOperator::Inequality | BinaryOperator::StrictInequality
            ) =>
        {
            (expression_is_ident(&binary.left, loop_var) && expression_is_string(&binary.right, "default"))
                || (expression_is_string(&binary.left, "default") && expression_is_ident(&binary.right, loop_var))
        }
        Expression::LogicalExpression(logical) => {
            expr_has_default_guard_for_loop_var(&logical.left, loop_var)
                || expr_has_default_guard_for_loop_var(&logical.right, loop_var)
        }
        Expression::ParenthesizedExpression(paren) => expr_has_default_guard_for_loop_var(&paren.expression, loop_var),
        _ => false,
    }
}

fn stmt_has_reexport_define_property_copy(
    stmt: &Statement<'_>,
    target_param: &str,
    source_param: &str,
    loop_var: &str,
    helpers: &BunHelperMap,
) -> bool {
    match stmt {
        Statement::ExpressionStatement(expr_stmt) => {
            expr_has_reexport_define_property_copy(&expr_stmt.expression, target_param, source_param, loop_var, helpers)
        }
        Statement::IfStatement(if_stmt) => {
            stmt_has_reexport_define_property_copy(&if_stmt.consequent, target_param, source_param, loop_var, helpers)
                || if_stmt.alternate.as_ref().is_some_and(|alternate| {
                    stmt_has_reexport_define_property_copy(alternate, target_param, source_param, loop_var, helpers)
                })
        }
        Statement::BlockStatement(block) => block
            .body
            .iter()
            .any(|stmt| stmt_has_reexport_define_property_copy(stmt, target_param, source_param, loop_var, helpers)),
        _ => false,
    }
}

fn expr_has_reexport_define_property_copy(
    expr: &Expression<'_>,
    target_param: &str,
    source_param: &str,
    loop_var: &str,
    helpers: &BunHelperMap,
) -> bool {
    match expr {
        Expression::CallExpression(call) => {
            is_define_property_copy_call(call, target_param, source_param, loop_var, helpers)
        }
        Expression::ParenthesizedExpression(paren) => {
            expr_has_reexport_define_property_copy(&paren.expression, target_param, source_param, loop_var, helpers)
        }
        Expression::SequenceExpression(seq) => seq
            .expressions
            .iter()
            .any(|expr| expr_has_reexport_define_property_copy(expr, target_param, source_param, loop_var, helpers)),
        Expression::LogicalExpression(logical) => {
            expr_has_reexport_define_property_copy(&logical.left, target_param, source_param, loop_var, helpers)
                || expr_has_reexport_define_property_copy(&logical.right, target_param, source_param, loop_var, helpers)
        }
        _ => false,
    }
}

fn is_define_property_copy_call(
    call: &CallExpression<'_>,
    target_param: &str,
    source_param: &str,
    loop_var: &str,
    helpers: &BunHelperMap,
) -> bool {
    if !call_callee_is_helper_or_object_property(call, helpers, BunHelper::DefProp, "defineProperty") {
        return false;
    }
    let Some(target_arg) = call.arguments.first() else {
        return false;
    };
    let Some(key_arg) = call.arguments.get(1) else {
        return false;
    };
    let Some(descriptor_arg) = call.arguments.get(2) else {
        return false;
    };

    argument_is_ident(target_arg, target_param)
        && argument_is_ident(key_arg, loop_var)
        && argument_descriptor_getter_reads_source_key(descriptor_arg, source_param, loop_var)
}

fn call_callee_is_helper_or_object_property(
    call: &CallExpression<'_>,
    helpers: &BunHelperMap,
    helper: BunHelper,
    object_property: &str,
) -> bool {
    match &call.callee {
        Expression::Identifier(id) => {
            id.name.as_str() == helper.canonical()
                || helpers
                    .get(id.name.as_str())
                    .is_some_and(|detected_helper| *detected_helper == helper)
        }
        Expression::StaticMemberExpression(member) => {
            matches!(&member.object, Expression::Identifier(obj) if obj.name.as_str() == "Object")
                && member.property.name.as_str() == object_property
        }
        _ => false,
    }
}

fn argument_descriptor_getter_reads_source_key(arg: &Argument<'_>, source_param: &str, loop_var: &str) -> bool {
    let Argument::ObjectExpression(obj) = arg else {
        return false;
    };
    obj.properties.iter().any(|property| {
        let ObjectPropertyKind::ObjectProperty(prop) = property else {
            return false;
        };
        prop.key.name().is_some_and(|name| name.as_ref() == "get")
            && expr_contains_source_key(&prop.value, source_param, loop_var)
    })
}

fn argument_contains_source_key(arg: &Argument<'_>, source_param: &str, loop_var: &str) -> bool {
    match arg {
        Argument::Identifier(id) => id.name.as_str() == source_param,
        Argument::ComputedMemberExpression(member) => {
            expression_is_ident(&member.object, source_param) && expression_is_ident(&member.expression, loop_var)
        }
        Argument::ObjectExpression(obj) => obj.properties.iter().any(|property| {
            let ObjectPropertyKind::ObjectProperty(prop) = property else {
                return false;
            };
            expr_contains_source_key(&prop.value, source_param, loop_var)
        }),
        _ => false,
    }
}

fn expr_contains_source_key(expr: &Expression<'_>, source_param: &str, loop_var: &str) -> bool {
    match expr {
        Expression::ComputedMemberExpression(member) => {
            expression_is_ident(&member.object, source_param) && expression_is_ident(&member.expression, loop_var)
        }
        Expression::ArrowFunctionExpression(arrow) => arrow
            .body
            .statements
            .iter()
            .any(|stmt| stmt_contains_source_key(stmt, source_param, loop_var)),
        Expression::FunctionExpression(func) => func.body.as_ref().is_some_and(|body| {
            body.statements
                .iter()
                .any(|stmt| stmt_contains_source_key(stmt, source_param, loop_var))
        }),
        Expression::ParenthesizedExpression(paren) => {
            expr_contains_source_key(&paren.expression, source_param, loop_var)
        }
        Expression::CallExpression(call) => {
            expr_contains_source_key(&call.callee, source_param, loop_var)
                || call
                    .arguments
                    .iter()
                    .any(|arg| argument_contains_source_key(arg, source_param, loop_var))
        }
        Expression::ObjectExpression(obj) => obj.properties.iter().any(|property| {
            let ObjectPropertyKind::ObjectProperty(prop) = property else {
                return false;
            };
            expr_contains_source_key(&prop.value, source_param, loop_var)
        }),
        Expression::BinaryExpression(binary) => {
            expr_contains_source_key(&binary.left, source_param, loop_var)
                || expr_contains_source_key(&binary.right, source_param, loop_var)
        }
        Expression::LogicalExpression(logical) => {
            expr_contains_source_key(&logical.left, source_param, loop_var)
                || expr_contains_source_key(&logical.right, source_param, loop_var)
        }
        Expression::ConditionalExpression(cond) => {
            expr_contains_source_key(&cond.test, source_param, loop_var)
                || expr_contains_source_key(&cond.consequent, source_param, loop_var)
                || expr_contains_source_key(&cond.alternate, source_param, loop_var)
        }
        _ => false,
    }
}

fn stmt_contains_source_key(stmt: &Statement<'_>, source_param: &str, loop_var: &str) -> bool {
    match stmt {
        Statement::ExpressionStatement(expr_stmt) => {
            expr_contains_source_key(&expr_stmt.expression, source_param, loop_var)
        }
        Statement::ReturnStatement(ret) => ret
            .argument
            .as_ref()
            .is_some_and(|arg| expr_contains_source_key(arg, source_param, loop_var)),
        Statement::IfStatement(if_stmt) => {
            expr_contains_source_key(&if_stmt.test, source_param, loop_var)
                || stmt_contains_source_key(&if_stmt.consequent, source_param, loop_var)
                || if_stmt
                    .alternate
                    .as_ref()
                    .is_some_and(|alternate| stmt_contains_source_key(alternate, source_param, loop_var))
        }
        Statement::BlockStatement(block) => block
            .body
            .iter()
            .any(|stmt| stmt_contains_source_key(stmt, source_param, loop_var)),
        _ => false,
    }
}

fn argument_is_ident(arg: &Argument<'_>, name: &str) -> bool {
    matches!(arg, Argument::Identifier(id) if id.name.as_str() == name)
}

fn expression_is_ident(expr: &Expression<'_>, name: &str) -> bool {
    matches!(expr, Expression::Identifier(id) if id.name.as_str() == name)
}

fn expression_is_string(expr: &Expression<'_>, value: &str) -> bool {
    matches!(expr, Expression::StringLiteral(lit) if lit.value.as_str() == value)
}

/// `__toESM`: `(mod, isNodeMode, target) => …`. 2-3 params, block body, contains __getProtoOf.
fn is_toesm_helper<'a>(arrow: &oxc_ast::ast::ArrowFunctionExpression<'a>) -> bool {
    let n = arrow.params.items.len();
    if !(2..=3).contains(&n) || arrow.expression {
        return false;
    }
    // Heuristic: body contains a reference to a getPrototypeOf-aliased function call,
    // OR contains an `__esModule` string check.
    body_contains_esmodule_check(&arrow.body.statements)
}

/// `__toCommonJS`: `from => { … __esModule: true … }`, 1 param.
fn is_tocommonjs_helper<'a>(arrow: &oxc_ast::ast::ArrowFunctionExpression<'a>) -> bool {
    if arrow.params.items.len() != 1 || arrow.expression {
        return false;
    }
    body_contains_esmodule_string(&arrow.body.statements)
}

/// `__name` arrow form: `(target, name) => { Object.defineProperty(target, "name", …); return target; }`
fn is_name_helper_arrow<'a>(arrow: &oxc_ast::ast::ArrowFunctionExpression<'a>) -> bool {
    if arrow.params.items.len() != 2 || arrow.expression {
        return false;
    }
    body_calls_define_property_with_name(&arrow.body.statements)
}

fn body_contains_esmodule_check<'a>(stmts: &[Statement<'a>]) -> bool {
    for stmt in stmts {
        if stmt_str_contains(stmt, "__esModule") {
            return true;
        }
    }
    false
}

fn body_contains_esmodule_string<'a>(stmts: &[Statement<'a>]) -> bool {
    body_contains_esmodule_check(stmts)
}

fn body_calls_define_property_with_name<'a>(stmts: &[Statement<'a>]) -> bool {
    // Look for ExpressionStatement -> CallExpression with one argument that's the string "name".
    for stmt in stmts {
        if let Statement::ExpressionStatement(es) = stmt {
            if call_has_string_arg(&es.expression, "name") {
                return true;
            }
        }
    }
    false
}

fn call_has_string_arg<'a>(expr: &Expression<'a>, target: &str) -> bool {
    let Expression::CallExpression(call) = expr else {
        return false;
    };
    call.arguments.iter().any(|a| {
        if let oxc_ast::ast::Argument::StringLiteral(s) = a {
            s.value.as_str() == target
        } else {
            false
        }
    })
}

/// Recursively check if a statement subtree contains a string literal matching `target`.
fn stmt_str_contains<'a>(stmt: &Statement<'a>, target: &str) -> bool {
    contains_str_in_stmt(stmt, target)
}

fn contains_str_in_stmt<'a>(stmt: &Statement<'a>, target: &str) -> bool {
    match stmt {
        Statement::ExpressionStatement(es) => contains_str_in_expr(&es.expression, target),
        Statement::ReturnStatement(rs) => rs.argument.as_ref().is_some_and(|e| contains_str_in_expr(e, target)),
        Statement::IfStatement(is) => {
            contains_str_in_expr(&is.test, target)
                || contains_str_in_stmt(&is.consequent, target)
                || is.alternate.as_ref().is_some_and(|s| contains_str_in_stmt(s, target))
        }
        Statement::BlockStatement(bs) => bs.body.iter().any(|s| contains_str_in_stmt(s, target)),
        Statement::VariableDeclaration(vd) => vd
            .declarations
            .iter()
            .any(|d| d.init.as_ref().is_some_and(|e| contains_str_in_expr(e, target))),
        Statement::ForStatement(fs) => contains_str_in_stmt(&fs.body, target),
        Statement::ForInStatement(fs) => contains_str_in_stmt(&fs.body, target),
        Statement::ForOfStatement(fs) => contains_str_in_stmt(&fs.body, target),
        _ => false,
    }
}

fn stmt_returns_ident(stmt: &Statement<'_>, target: &str) -> bool {
    match stmt {
        Statement::ReturnStatement(ret) => {
            matches!(&ret.argument, Some(Expression::Identifier(id)) if id.name.as_str() == target)
        }
        Statement::BlockStatement(block) => block.body.iter().any(|stmt| stmt_returns_ident(stmt, target)),
        Statement::IfStatement(if_stmt) => {
            stmt_returns_ident(&if_stmt.consequent, target)
                || if_stmt
                    .alternate
                    .as_ref()
                    .is_some_and(|alternate| stmt_returns_ident(alternate, target))
        }
        _ => false,
    }
}

fn contains_str_in_expr<'a>(expr: &Expression<'a>, target: &str) -> bool {
    match expr {
        Expression::StringLiteral(s) => s.value.as_str() == target,
        Expression::ParenthesizedExpression(p) => contains_str_in_expr(&p.expression, target),
        Expression::SequenceExpression(s) => s.expressions.iter().any(|e| contains_str_in_expr(e, target)),
        Expression::CallExpression(c) => {
            contains_str_in_expr(&c.callee, target)
                || c.arguments.iter().any(|a| match a {
                    oxc_ast::ast::Argument::StringLiteral(s) => s.value.as_str() == target,
                    _ => false,
                })
        }
        Expression::ObjectExpression(o) => o.properties.iter().any(|p| match p {
            oxc_ast::ast::ObjectPropertyKind::ObjectProperty(prop) => contains_str_in_expr(&prop.value, target),
            _ => false,
        }),
        Expression::AssignmentExpression(a) => contains_str_in_expr(&a.right, target),
        Expression::BinaryExpression(b) => {
            contains_str_in_expr(&b.left, target) || contains_str_in_expr(&b.right, target)
        }
        Expression::LogicalExpression(l) => {
            contains_str_in_expr(&l.left, target) || contains_str_in_expr(&l.right, target)
        }
        Expression::ConditionalExpression(c) => {
            contains_str_in_expr(&c.test, target)
                || contains_str_in_expr(&c.consequent, target)
                || contains_str_in_expr(&c.alternate, target)
        }
        Expression::StaticMemberExpression(m) => {
            contains_str_in_expr(&m.object, target) || m.property.name.as_str() == target
        }
        Expression::UnaryExpression(u) => contains_str_in_expr(&u.argument, target),
        _ => false,
    }
}

/// Unwraps the "single ExpressionStatement returning an arrow whose body is a single
/// ExpressionStatement returning a paren-wrapped sequence" structure used by both
/// `__esm` and `__commonJS`. Returns the inner-most expression after paren-unwrap.
fn inner_arrow_body_expr<'a, 'b>(arrow: &'a oxc_ast::ast::ArrowFunctionExpression<'b>) -> Option<&'a Expression<'b>> {
    let outer_stmt = arrow.body.statements.first()?;
    let Statement::ExpressionStatement(es) = outer_stmt else {
        return None;
    };
    let Expression::ArrowFunctionExpression(inner_arrow) = &es.expression else {
        return None;
    };
    if !inner_arrow.params.items.is_empty() || !inner_arrow.expression {
        return None;
    }
    let inner_stmt = inner_arrow.body.statements.first()?;
    let Statement::ExpressionStatement(inner_es) = inner_stmt else {
        return None;
    };
    let unwrap = match &inner_es.expression {
        Expression::ParenthesizedExpression(p) => &p.expression,
        other => other,
    };
    Some(unwrap)
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

    fn collect(code: &str) -> BunHelperMap {
        let alloc = Allocator::default();
        let parsed = Parser::new(&alloc, code, SourceType::mjs()).parse();
        let mut prog = parsed.program;
        let mut collector = BunRuntimeCollector::new();
        let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &alloc);
        traverse_mut_with_ctx(&mut collector, &mut prog, &mut ctx);
        collector.into_map()
    }

    fn collect_and_rename(code: &str) -> (String, BunHelperMap) {
        let alloc = Allocator::default();
        let parsed = Parser::new(&alloc, code, SourceType::mjs()).parse();
        let mut prog = parsed.program;

        let mut collector = BunRuntimeCollector::new();
        let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &alloc);
        traverse_mut_with_ctx(&mut collector, &mut prog, &mut ctx);
        let map = collector.into_map();

        let mut renamer = BunRuntimeRenamer::new(map.clone());
        let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &alloc);
        traverse_mut_with_ctx(&mut renamer, &mut prog, &mut ctx);

        let out = Codegen::new().build(&prog).code;
        (out, map)
    }

    #[test]
    fn detects_esm_helper() {
        let map = collect("var V = (H, $) => () => (H && ($ = H(H = 0)), $);");
        assert_eq!(map.get("V"), Some(&BunHelper::Esm));
    }

    #[test]
    fn detects_commonjs_helper() {
        let map = collect("var n = (H, $) => () => ($ || H(($ = { exports: {} }).exports, $), $.exports);");
        assert_eq!(map.get("n"), Some(&BunHelper::CommonJs));
    }

    #[test]
    fn detects_reexport_before_copyprops() {
        let map = collect(
            r#"
            var __defProp = Object.defineProperty;
            var __getOwnPropNames = Object.getOwnPropertyNames;
            var R = (H, $, q) => {
                var K = __getOwnPropNames($);
                for (let _ of K) if (_ !== "default") __defProp(H, _, { get: () => $[_], enumerable: true });
                return q;
            };
            "#,
        );
        assert_eq!(map.get("R"), Some(&BunHelper::ReExport));
    }

    #[test]
    fn does_not_detect_broad_reexport_lookalike() {
        let map = collect(
            r#"
            var R = (H, $, q) => {
                var K = list($);
                for (let _ of K) if (_ !== "default") H[_] = $[_];
                return q;
            };
            "#,
        );
        assert_eq!(map.get("R"), None);
    }

    #[test]
    fn detects_copyprops_with_get_own_property_names_loop() {
        let map = collect(
            r#"
            var C = (H, $, q) => {
                var K = Object.getOwnPropertyNames($);
                for (let _ of K) H[_] = $[_];
            };
            "#,
        );
        assert_eq!(map.get("C"), Some(&BunHelper::CopyProps));
    }

    #[test]
    fn detects_destructured_object_aliases() {
        let map =
            collect("var { defineProperty: fjH, getOwnPropertyNames: UP8, getOwnPropertyDescriptor: Cd4 } = Object;");
        assert_eq!(map.get("fjH"), Some(&BunHelper::DefProp));
        assert_eq!(map.get("UP8"), Some(&BunHelper::GetOwnPropNames));
        assert_eq!(map.get("Cd4"), Some(&BunHelper::GetOwnPropDesc));
    }

    #[test]
    fn detects_create_alias() {
        let map = collect("var Sd4 = Object.create;");
        assert_eq!(map.get("Sd4"), Some(&BunHelper::Create));
    }

    #[test]
    fn detects_hasownprop() {
        let map = collect("var XG$ = Object.prototype.hasOwnProperty;");
        assert_eq!(map.get("XG$"), Some(&BunHelper::HasOwnProp));
    }

    #[test]
    fn detects_accessprop_function() {
        let map = collect("function JG$(H) { return this[H]; }");
        assert_eq!(map.get("JG$"), Some(&BunHelper::AccessProp));
    }

    #[test]
    fn descends_into_cjs_wrapper_iife() {
        let code = r#"
            (function(exports, require, module, __filename, __dirname) {
                var V = (H, $) => () => (H && ($ = H(H = 0)), $);
                var n = (H, $) => () => ($ || H(($ = { exports: {} }).exports, $), $.exports);
                var Sd4 = Object.create;
            })(exports, require, module, __filename, __dirname);
        "#;
        let map = collect(code);
        assert_eq!(map.get("V"), Some(&BunHelper::Esm));
        assert_eq!(map.get("n"), Some(&BunHelper::CommonJs));
        assert_eq!(map.get("Sd4"), Some(&BunHelper::Create));
    }

    #[test]
    fn renames_references() {
        let code = r#"
            var V = (H, $) => () => (H && ($ = H(H = 0)), $);
            var modA = V(() => { console.log("a"); });
            var modB = V(() => { console.log("b"); });
        "#;
        let (out, _) = collect_and_rename(code);
        assert!(out.contains("__esm"), "Should rename V to __esm: {out}");
        assert!(!out.contains(" V("), "Should not retain V() calls: {out}");
        // The decl should also be renamed.
        assert!(out.contains("var __esm ="));
    }

    #[test]
    fn renames_destructured_aliases() {
        let code = r#"
            var { defineProperty: fjH } = Object;
            fjH(target, "x", { value: 1 });
        "#;
        let (out, _) = collect_and_rename(code);
        assert!(out.contains("__defProp"), "Should rename fjH to __defProp: {out}");
        assert!(!out.contains("fjH"), "Should not retain fjH: {out}");
    }
}
