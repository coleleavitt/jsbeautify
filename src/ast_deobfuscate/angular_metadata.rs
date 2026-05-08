//! Angular metadata extractor and component renamer.
//!
//! Identifies Angular compiled output patterns and extracts:
//! - Component selectors from `ɵɵdefineComponent({selectors: [["my-comp"]]})`
//! - Directive selectors from `ɵɵdefineDirective({selectors: ...})`
//! - Injectable/service definitions from `ɵɵdefineInjectable({...})`
//! - NgModule definitions from `ɵɵdefineNgModule({type: X})`
//! - Pipe definitions from `ɵɵdefinePipe({name: "myPipe", ...})`
//!
//! Uses extracted selectors to rename minified class variables to meaningful names:
//!   `lms-app-root` → class renamed to `LmsAppRootComponent`
//!   `lms-icon` → class renamed to `LmsIconComponent`
//!
//! Angular compiler output pattern (v10-v19):
//! ```js
//! class d { constructor() { ... } ngOnInit() { ... } }
//! let Ss = ((l = d).ɵfac = function(a) { ... },
//!           l.ɵcmp = ɵɵdefineComponent({type: l, selectors: [["lms-login"]], ...}),
//!           d);
//! ```
//!
//! Supports Angular versions 10.x through 19.x compiled output.

use oxc_ast::ast::{
    ArrayExpressionElement, CallExpression, Expression, ObjectPropertyKind, PropertyKey, Statement, VariableDeclarator,
};
use rustc_hash::FxHashMap;

/// Metadata extracted from a single Angular definition call.
#[derive(Debug, Clone)]
pub struct AngularComponentInfo {
    /// The CSS selector (e.g. "lms-app-root", "[myDirective]", "option")
    pub selector: String,
    /// Generated PascalCase name (e.g. "LmsAppRootComponent")
    pub class_name: String,
    /// The kind of Angular artifact
    pub kind: AngularDefKind,
    /// Inputs declared on this component/directive
    pub inputs: Vec<String>,
    /// Outputs declared on this component/directive
    pub outputs: Vec<String>,
    /// The minified variable name that holds this class (e.g. "Ss", "Fe")
    pub minified_export_name: Option<String>,
    /// The minified class declaration name (e.g. "d", "Le")
    pub minified_class_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AngularDefKind {
    Component,
    Directive,
    Pipe,
    Injectable,
    NgModule,
}

impl AngularDefKind {
    fn suffix(self) -> &'static str {
        match self {
            Self::Component => "Component",
            Self::Directive => "Directive",
            Self::Pipe => "Pipe",
            Self::Injectable => "Service",
            Self::NgModule => "Module",
        }
    }
}

/// Extracts Angular metadata from beautified/deobfuscated JS output.
///
/// Works on the string level (regex + oxc AST) because the patterns are
/// very consistent across Angular compiler versions.
pub struct AngularMetadataExtractor {
    /// All discovered Angular definitions, keyed by minified class name
    pub definitions: Vec<AngularComponentInfo>,
    /// Map from minified name → generated readable name
    pub rename_map: FxHashMap<String, String>,
}

impl Default for AngularMetadataExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl AngularMetadataExtractor {
    #[must_use]
    pub fn new() -> Self {
        Self {
            definitions: Vec::new(),
            rename_map: FxHashMap::default(),
        }
    }

    /// Run extraction on a parsed AST program.
    pub fn extract_from_statements(&mut self, statements: &[Statement<'_>]) {
        for stmt in statements {
            self.visit_statement(stmt);
        }
        self.build_rename_map();
    }

    /// Recursively walk a statement looking for Angular define calls at any depth.
    fn visit_statement(&mut self, stmt: &Statement<'_>) {
        match stmt {
            Statement::VariableDeclaration(var_decl) => {
                for decl in &var_decl.declarations {
                    self.check_variable_declarator(decl);
                    // Recurse into initializer for nested functions/arrows that
                    // might contain OTHER components (webpack module callbacks).
                    // But don't double-process the define call — walk_expression
                    // only recurses into function bodies, not the top-level expr.
                    if let Some(init) = &decl.init {
                        self.walk_into_functions_only(init);
                    }
                }
            }
            Statement::ExpressionStatement(expr_stmt) => {
                self.check_expression_for_define(&expr_stmt.expression, None);
                self.walk_expression(&expr_stmt.expression);
            }
            Statement::BlockStatement(block) => {
                for s in &block.body {
                    self.visit_statement(s);
                }
            }
            Statement::IfStatement(if_stmt) => {
                self.visit_statement(&if_stmt.consequent);
                if let Some(alt) = &if_stmt.alternate {
                    self.visit_statement(alt);
                }
            }
            Statement::ForStatement(for_stmt) => {
                self.visit_statement(&for_stmt.body);
            }
            Statement::WhileStatement(while_stmt) => {
                self.visit_statement(&while_stmt.body);
            }
            Statement::DoWhileStatement(do_while) => {
                self.visit_statement(&do_while.body);
            }
            Statement::ReturnStatement(ret) => {
                if let Some(arg) = &ret.argument {
                    self.walk_expression(arg);
                }
            }
            Statement::TryStatement(try_stmt) => {
                for s in &try_stmt.block.body {
                    self.visit_statement(s);
                }
                if let Some(handler) = &try_stmt.handler {
                    for s in &handler.body.body {
                        self.visit_statement(s);
                    }
                }
                if let Some(finalizer) = &try_stmt.finalizer {
                    for s in &finalizer.body {
                        self.visit_statement(s);
                    }
                }
            }
            Statement::SwitchStatement(switch) => {
                for case in &switch.cases {
                    for s in &case.consequent {
                        self.visit_statement(s);
                    }
                }
            }
            _ => {}
        }
    }

    /// Walk into expressions to find Angular defines inside nested functions/arrows/objects.
    fn walk_expression(&mut self, expr: &Expression<'_>) {
        match expr {
            Expression::FunctionExpression(func) => {
                if let Some(body) = &func.body {
                    for stmt in &body.statements {
                        self.visit_statement(stmt);
                    }
                }
            }
            Expression::ArrowFunctionExpression(arrow) => {
                for stmt in &arrow.body.statements {
                    self.visit_statement(stmt);
                }
            }
            Expression::CallExpression(call) => {
                // Walk into arguments (webpack callbacks etc.)
                for arg in &call.arguments {
                    if let Some(expr) = arg.as_expression() {
                        self.walk_expression(expr);
                    }
                }
                self.walk_expression(&call.callee);
            }
            Expression::ObjectExpression(obj) => {
                for prop in &obj.properties {
                    if let ObjectPropertyKind::ObjectProperty(p) = prop {
                        self.walk_expression(&p.value);
                    }
                }
            }
            Expression::ArrayExpression(arr) => {
                for elem in &arr.elements {
                    match elem {
                        ArrayExpressionElement::SpreadElement(spread) => {
                            self.walk_expression(&spread.argument);
                        }
                        ArrayExpressionElement::Elision(_) => {}
                        _ => {
                            // All other variants are expression types
                            if let Some(expr) = elem.as_expression() {
                                self.walk_expression(expr);
                            }
                        }
                    }
                }
            }
            Expression::SequenceExpression(seq) => {
                for sub in &seq.expressions {
                    self.check_expression_for_define(sub, None);
                    self.walk_expression(sub);
                }
            }
            Expression::AssignmentExpression(assign) => {
                self.walk_expression(&assign.right);
            }
            Expression::ParenthesizedExpression(paren) => {
                self.walk_expression(&paren.expression);
            }
            Expression::ConditionalExpression(cond) => {
                self.walk_expression(&cond.consequent);
                self.walk_expression(&cond.alternate);
            }
            _ => {}
        }
    }

    /// Like walk_expression but only descends into function/arrow bodies.
    /// Used for variable initializers where check_expression_for_define already
    /// handles the top-level sequence/assignment, avoiding double-detection.
    fn walk_into_functions_only(&mut self, expr: &Expression<'_>) {
        match expr {
            Expression::FunctionExpression(func) => {
                if let Some(body) = &func.body {
                    for stmt in &body.statements {
                        self.visit_statement(stmt);
                    }
                }
            }
            Expression::ArrowFunctionExpression(arrow) => {
                for stmt in &arrow.body.statements {
                    self.visit_statement(stmt);
                }
            }
            Expression::CallExpression(call) => {
                for arg in &call.arguments {
                    if let Some(expr) = arg.as_expression() {
                        self.walk_into_functions_only(expr);
                    }
                }
                self.walk_into_functions_only(&call.callee);
            }
            Expression::ObjectExpression(obj) => {
                for prop in &obj.properties {
                    if let ObjectPropertyKind::ObjectProperty(p) = prop {
                        self.walk_into_functions_only(&p.value);
                    }
                }
            }
            Expression::ArrayExpression(arr) => {
                for elem in &arr.elements {
                    if let Some(expr) = elem.as_expression() {
                        self.walk_into_functions_only(expr);
                    }
                }
            }
            Expression::ParenthesizedExpression(paren) => {
                self.walk_into_functions_only(&paren.expression);
            }
            _ => {}
        }
    }

    /// After extraction, annotate the source code with component boundary comments.
    /// Returns the annotated source and the count of annotations added.
    #[must_use]
    pub fn annotate_source(&self, source: &str) -> (String, usize) {
        if self.definitions.is_empty() {
            return (source.to_string(), 0);
        }

        let mut result = String::with_capacity(source.len() + self.definitions.len() * 80);
        let mut annotations = 0usize;

        for line in source.lines() {
            let trimmed = line.trim();

            // Detect component definition lines
            if let Some(info) = self.match_definition_line(trimmed) {
                let kind_label = match info.kind {
                    AngularDefKind::Component => "Component",
                    AngularDefKind::Directive => "Directive",
                    AngularDefKind::Pipe => "Pipe",
                    AngularDefKind::Injectable => "Injectable",
                    AngularDefKind::NgModule => "NgModule",
                };

                result.push_str("// ─── Angular ");
                result.push_str(kind_label);
                result.push_str(": ");
                result.push_str(&info.class_name);
                if !info.selector.is_empty() {
                    result.push_str(" (selector: \"");
                    result.push_str(&info.selector);
                    result.push_str("\")");
                }
                if !info.inputs.is_empty() {
                    result.push_str(" inputs: [");
                    result.push_str(&info.inputs.join(", "));
                    result.push(']');
                }
                if !info.outputs.is_empty() {
                    result.push_str(" outputs: [");
                    result.push_str(&info.outputs.join(", "));
                    result.push(']');
                }
                result.push_str(" ───\n");
                annotations += 1;
            }

            result.push_str(line);
            result.push('\n');
        }

        (result, annotations)
    }

    /// Apply rename map to source code — replaces minified class names with generated names.
    /// Returns (new_source, rename_count).
    #[must_use]
    pub fn apply_renames(&self, source: &str) -> (String, usize) {
        if self.rename_map.is_empty() {
            return (source.to_string(), 0);
        }

        let mut result = source.to_string();
        let mut count = 0usize;

        // Sort by length descending to avoid partial replacements
        let mut renames: Vec<(&String, &String)> = self.rename_map.iter().collect();
        renames.sort_by(|a, b| b.0.len().cmp(&a.0.len()));

        for (minified, readable) in renames {
            // Only rename if it appears as a word boundary (not inside another identifier)
            let pattern = format!(r"\b{}\b", regex::escape(minified));
            if let Ok(re) = regex::Regex::new(&pattern) {
                let new_result = re.replace_all(&result, readable.as_str()).to_string();
                if new_result != result {
                    count += 1;
                    result = new_result;
                }
            }
        }

        (result, count)
    }

    fn build_rename_map(&mut self) {
        for info in &self.definitions {
            if let Some(ref export_name) = info.minified_export_name {
                // Only rename short minified names (1-3 chars are definitely minified)
                if export_name.len() <= 3 && !info.class_name.is_empty() {
                    self.rename_map.insert(export_name.clone(), info.class_name.clone());
                }
            }
            if let Some(ref class_name) = info.minified_class_name {
                if class_name.len() <= 3 && !info.class_name.is_empty() {
                    self.rename_map.insert(class_name.clone(), info.class_name.clone());
                }
            }
        }
    }

    fn match_definition_line<'a>(&'a self, line: &str) -> Option<&'a AngularComponentInfo> {
        for info in &self.definitions {
            // Match on the selector appearing in a defineComponent/defineDirective call
            if line.contains("defineComponent") || line.contains("defineDirective") {
                if let Some(ref sel) = Some(&info.selector) {
                    if !sel.is_empty() && line.contains(sel.as_str()) {
                        return Some(info);
                    }
                }
            }
            if line.contains("definePipe") && line.contains(&info.selector) {
                return Some(info);
            }
        }
        None
    }

    fn check_variable_declarator(&mut self, decl: &VariableDeclarator<'_>) {
        // Pattern: let Ss = ((l = d).ɵfac = ..., l.ɵcmp = ɵɵdefineComponent({...}), d)
        let export_name = decl.id.get_identifier_name().map(|a| a.to_string());

        if let Some(init) = &decl.init {
            self.check_expression_for_define(init, export_name.as_deref());
        }
    }

    fn check_expression_for_define(&mut self, expr: &Expression<'_>, export_name: Option<&str>) {
        match expr {
            Expression::SequenceExpression(seq) => {
                for sub_expr in &seq.expressions {
                    self.check_expression_for_define(sub_expr, export_name);
                }
            }
            Expression::AssignmentExpression(assign) => {
                // l.ɵcmp = ɵɵdefineComponent({...})
                self.check_expression_for_define(&assign.right, export_name);
            }
            Expression::CallExpression(call) => {
                self.check_call_for_define(call, export_name);
            }
            Expression::ParenthesizedExpression(paren) => {
                self.check_expression_for_define(&paren.expression, export_name);
            }
            _ => {}
        }
    }

    fn check_call_for_define(&mut self, call: &CallExpression<'_>, export_name: Option<&str>) {
        let callee_name = self.get_callee_name(call);
        let Some(callee) = callee_name else { return };

        let kind = match callee.as_str() {
            name if name.contains("defineComponent") => AngularDefKind::Component,
            name if name.contains("defineDirective") => AngularDefKind::Directive,
            name if name.contains("definePipe") => AngularDefKind::Pipe,
            name if name.contains("defineInjectable") => AngularDefKind::Injectable,
            name if name.contains("defineNgModule") => AngularDefKind::NgModule,
            _ => return,
        };

        // Extract metadata from the first object argument
        let Some(first_arg) = call.arguments.first() else {
            return;
        };
        let Some(arg_expr) = first_arg.as_expression() else {
            return;
        };

        let Expression::ObjectExpression(obj) = arg_expr else {
            return;
        };

        let mut selector = String::new();
        let mut inputs: Vec<String> = Vec::new();
        let mut outputs: Vec<String> = Vec::new();
        let mut pipe_name = String::new();

        for prop in &obj.properties {
            let ObjectPropertyKind::ObjectProperty(prop) = prop else {
                continue;
            };

            let key_name = match &prop.key {
                PropertyKey::StaticIdentifier(ident) => ident.name.as_str().to_string(),
                PropertyKey::StringLiteral(lit) => lit.value.to_string(),
                _ => continue,
            };

            match key_name.as_str() {
                "selectors" => {
                    // selectors: [["my-component"]] or selectors: [["", "myDirective", ""]]
                    selector = self.extract_selector(&prop.value);
                }
                "inputs" => {
                    inputs = self.extract_string_map_keys(&prop.value);
                }
                "outputs" => {
                    outputs = self.extract_string_map_keys(&prop.value);
                }
                "name" if kind == AngularDefKind::Pipe => {
                    // Pipe name is in `name: "myPipe"`
                    if let Expression::StringLiteral(lit) = &prop.value {
                        pipe_name = lit.value.to_string();
                    }
                }
                _ => {}
            }
        }

        // For pipes, use the pipe name as the selector
        if kind == AngularDefKind::Pipe && !pipe_name.is_empty() {
            selector = pipe_name;
        }

        let class_name = generate_class_name(&selector, kind);

        let info = AngularComponentInfo {
            selector,
            class_name,
            kind,
            inputs,
            outputs,
            minified_export_name: export_name.map(String::from),
            minified_class_name: None,
        };

        self.definitions.push(info);
    }

    fn get_callee_name(&self, call: &CallExpression<'_>) -> Option<String> {
        match &call.callee {
            Expression::Identifier(ident) => Some(ident.name.to_string()),
            Expression::StaticMemberExpression(member) => Some(member.property.name.to_string()),
            _ => None,
        }
    }

    /// Extract selector string from `selectors: [["my-comp"]]` or `selectors: [["", "dir", ""]]`
    fn extract_selector(&self, expr: &Expression<'_>) -> String {
        let Expression::ArrayExpression(outer) = expr else {
            return String::new();
        };

        // First element is the selector array
        let Some(first) = outer.elements.first() else {
            return String::new();
        };

        let ArrayExpressionElement::ArrayExpression(inner) = first else {
            return String::new();
        };

        // Collect non-empty strings from the inner array
        let mut parts: Vec<String> = Vec::new();
        for elem in &inner.elements {
            if let ArrayExpressionElement::StringLiteral(lit) = elem {
                let s = lit.value.to_string();
                if !s.is_empty() {
                    parts.push(s);
                }
            }
        }

        parts.join("")
    }

    /// Extract keys from an inputs/outputs object like `{foo: "foo", bar: "bar"}`
    /// or an array like `["foo", "bar"]`
    fn extract_string_map_keys(&self, expr: &Expression<'_>) -> Vec<String> {
        let mut keys = Vec::new();

        match expr {
            Expression::ObjectExpression(obj) => {
                for prop in &obj.properties {
                    if let ObjectPropertyKind::ObjectProperty(p) = prop {
                        match &p.key {
                            PropertyKey::StaticIdentifier(ident) => {
                                keys.push(ident.name.to_string());
                            }
                            PropertyKey::StringLiteral(lit) => {
                                keys.push(lit.value.to_string());
                            }
                            _ => {}
                        }
                    }
                }
            }
            Expression::ArrayExpression(arr) => {
                for elem in &arr.elements {
                    if let ArrayExpressionElement::StringLiteral(lit) = elem {
                        keys.push(lit.value.to_string());
                    }
                }
            }
            _ => {}
        }

        keys
    }
}

/// Convert a CSS selector to PascalCase class name.
/// - `lms-app-root` → `LmsAppRootComponent`
/// - `[myDirective]` → `MyDirectiveDirective`
/// - `option` → `OptionComponent`
/// - `router-outlet` → `RouterOutletComponent`
fn generate_class_name(selector: &str, kind: AngularDefKind) -> String {
    if selector.is_empty() {
        return String::new();
    }

    // Strip attribute selector brackets
    let clean = selector
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_start_matches('.');

    // Split on hyphens, dots, underscores
    let parts: Vec<&str> = clean.split(|c: char| c == '-' || c == '.' || c == '_').collect();

    let mut name = String::new();
    for part in parts {
        if !part.is_empty() {
            // Capitalize first letter
            let mut chars = part.chars();
            if let Some(first) = chars.next() {
                name.extend(first.to_uppercase());
                name.push_str(chars.as_str());
            }
        }
    }

    // Append the kind suffix if not already present
    let suffix = kind.suffix();
    if !name.ends_with(suffix) {
        name.push_str(suffix);
    }

    name
}

/// Extract Angular version fingerprint from compiled output.
/// Different Angular versions produce slightly different patterns:
/// - v10-v12: `dt.ɵcmp = ...` with `ɵɵ` prefix on all instructions
/// - v13+: same but with standalone component support
/// - v17+: `@defer`, `ɵɵconditional`, signals support
/// - v19+: `ɵɵdeclareLet`, resource API
#[must_use]
pub fn detect_angular_version(source: &str) -> Option<AngularVersionRange> {
    // Check for v19+ features
    if source.contains("ɵɵdeclareLet") || source.contains("ɵɵresource") {
        return Some(AngularVersionRange::V19Plus);
    }

    // Check for v17+ features (defer, control flow)
    if source.contains("ɵɵdefer") || source.contains("ɵɵconditional") {
        return Some(AngularVersionRange::V17To18);
    }

    // Check for v14+ standalone components
    if source.contains("standalone: true") || source.contains("standalone:!0") {
        return Some(AngularVersionRange::V14To16);
    }

    // Check for v13+ (ViewEncapsulation.ShadowDom as number)
    if source.contains("encapsulation: 3") {
        return Some(AngularVersionRange::V13);
    }

    // Basic Angular detection (v10-v12)
    if source.contains("ɵɵdefineComponent") || source.contains("ɵɵdefineDirective") {
        return Some(AngularVersionRange::V10To12);
    }

    // AngularJS (v1.x) — completely different pattern
    if source.contains("angular.module") || source.contains("$scope") {
        return Some(AngularVersionRange::AngularJS);
    }

    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AngularVersionRange {
    /// AngularJS 1.x (angular.module, $scope, etc.)
    AngularJS,
    /// Angular 10-12 (Ivy but no standalone)
    V10To12,
    /// Angular 13 (new ViewEncapsulation values)
    V13,
    /// Angular 14-16 (standalone components)
    V14To16,
    /// Angular 17-18 (defer blocks, new control flow)
    V17To18,
    /// Angular 19+ (declareLet, resource, signals)
    V19Plus,
}

impl std::fmt::Display for AngularVersionRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AngularJS => write!(f, "AngularJS 1.x"),
            Self::V10To12 => write!(f, "Angular 10-12"),
            Self::V13 => write!(f, "Angular 13"),
            Self::V14To16 => write!(f, "Angular 14-16"),
            Self::V17To18 => write!(f, "Angular 17-18"),
            Self::V19Plus => write!(f, "Angular 19+"),
        }
    }
}

/// High-level: run full Angular extraction on source text.
/// Returns (annotated_source, stats).
#[must_use]
pub fn extract_angular_metadata(source: &str) -> AngularExtractionResult {
    let version = detect_angular_version(source);

    let allocator = oxc_allocator::Allocator::new();
    let source_type = oxc_span::SourceType::default();
    let parsed = oxc_parser::Parser::new(&allocator, source, source_type).parse();

    let mut extractor = AngularMetadataExtractor::new();

    if parsed.errors.is_empty() && !parsed.program.body.is_empty() {
        // AST-based extraction (preferred — gives inputs/outputs/full metadata)
        extractor.extract_from_statements(&parsed.program.body);
    }

    // If AST extraction found nothing (parse errors, empty body, etc.),
    // fall back to regex-based extraction which works on any text.
    if extractor.definitions.is_empty() && source.contains("defineComponent") {
        eprintln!("[ANGULAR] AST parse failed or found nothing, using regex fallback");
        extract_via_regex(source, &mut extractor);
    }

    let component_count = extractor
        .definitions
        .iter()
        .filter(|d| d.kind == AngularDefKind::Component)
        .count();
    let directive_count = extractor
        .definitions
        .iter()
        .filter(|d| d.kind == AngularDefKind::Directive)
        .count();
    let pipe_count = extractor
        .definitions
        .iter()
        .filter(|d| d.kind == AngularDefKind::Pipe)
        .count();
    let injectable_count = extractor
        .definitions
        .iter()
        .filter(|d| d.kind == AngularDefKind::Injectable)
        .count();
    let module_count = extractor
        .definitions
        .iter()
        .filter(|d| d.kind == AngularDefKind::NgModule)
        .count();

    let (annotated, annotation_count) = extractor.annotate_source(source);

    AngularExtractionResult {
        source: annotated,
        version,
        component_count,
        directive_count,
        pipe_count,
        injectable_count,
        module_count,
        annotation_count,
        definitions: extractor.definitions,
        rename_map: extractor.rename_map,
    }
}

#[derive(Debug)]
pub struct AngularExtractionResult {
    pub source: String,
    pub version: Option<AngularVersionRange>,
    pub component_count: usize,
    pub directive_count: usize,
    pub pipe_count: usize,
    pub injectable_count: usize,
    pub module_count: usize,
    pub annotation_count: usize,
    pub definitions: Vec<AngularComponentInfo>,
    pub rename_map: FxHashMap<String, String>,
}

/// Regex-based fallback extraction for when AST parsing fails
/// (e.g. beautified output has duplicate identifiers across inlined modules).
fn extract_via_regex(source: &str, extractor: &mut AngularMetadataExtractor) {
    use regex::Regex;

    lazy_static::lazy_static! {
        // Matches: selectors: [["component-selector"]]
        static ref SELECTOR_RE: Regex = Regex::new(
            r#"(?:defineComponent|defineDirective)\(\{[^}]*?selectors:\s*\[\["([^"]+)"\]\]"#
        ).unwrap();

        // Matches: definePipe({name: "pipeName", ...})
        static ref PIPE_RE: Regex = Regex::new(
            r#"definePipe\(\{[^}]*?name:\s*"([^"]+)""#
        ).unwrap();

        // Matches: defineInjectable({...})
        static ref INJECTABLE_RE: Regex = Regex::new(
            r"defineInjectable\(\{"
        ).unwrap();

        // Matches: defineNgModule({...})
        static ref NGMODULE_RE: Regex = Regex::new(
            r"defineNgModule\(\{"
        ).unwrap();

        // Matches inputs: { key: "value", ... } inside defineComponent
        static ref INPUTS_RE: Regex = Regex::new(
            r#"defineComponent\(\{[^}]*?inputs:\s*\{([^}]*)\}"#
        ).unwrap();
    }

    // Extract components/directives via selectors
    for caps in SELECTOR_RE.captures_iter(source) {
        let selector = caps.get(1).map_or("", |m| m.as_str()).to_string();
        let full_match = caps.get(0).map_or("", |m| m.as_str());
        let kind = if full_match.contains("defineDirective") {
            AngularDefKind::Directive
        } else {
            AngularDefKind::Component
        };
        let class_name = generate_class_name(&selector, kind);

        extractor.definitions.push(AngularComponentInfo {
            selector,
            class_name,
            kind,
            inputs: Vec::new(),
            outputs: Vec::new(),
            minified_export_name: None,
            minified_class_name: None,
        });
    }

    // Extract pipes
    for caps in PIPE_RE.captures_iter(source) {
        let pipe_name = caps.get(1).map_or("", |m| m.as_str()).to_string();
        let class_name = generate_class_name(&pipe_name, AngularDefKind::Pipe);

        extractor.definitions.push(AngularComponentInfo {
            selector: pipe_name,
            class_name,
            kind: AngularDefKind::Pipe,
            inputs: Vec::new(),
            outputs: Vec::new(),
            minified_export_name: None,
            minified_class_name: None,
        });
    }

    // Count injectables
    for _ in INJECTABLE_RE.find_iter(source) {
        extractor.definitions.push(AngularComponentInfo {
            selector: String::new(),
            class_name: String::new(),
            kind: AngularDefKind::Injectable,
            inputs: Vec::new(),
            outputs: Vec::new(),
            minified_export_name: None,
            minified_class_name: None,
        });
    }

    // Count NgModules
    for _ in NGMODULE_RE.find_iter(source) {
        extractor.definitions.push(AngularComponentInfo {
            selector: String::new(),
            class_name: String::new(),
            kind: AngularDefKind::NgModule,
            inputs: Vec::new(),
            outputs: Vec::new(),
            minified_export_name: None,
            minified_class_name: None,
        });
    }

    extractor.build_rename_map();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_class_name_component() {
        assert_eq!(
            generate_class_name("lms-app-root", AngularDefKind::Component),
            "LmsAppRootComponent"
        );
        assert_eq!(
            generate_class_name("lms-icon", AngularDefKind::Component),
            "LmsIconComponent"
        );
        assert_eq!(
            generate_class_name("router-outlet", AngularDefKind::Component),
            "RouterOutletComponent"
        );
        assert_eq!(
            generate_class_name("option", AngularDefKind::Component),
            "OptionComponent"
        );
    }

    #[test]
    fn test_generate_class_name_directive() {
        assert_eq!(
            generate_class_name("myDirective", AngularDefKind::Directive),
            "MyDirective"
        );
        assert_eq!(
            generate_class_name("[ngClass]", AngularDefKind::Directive),
            "NgClassDirective"
        );
        assert_eq!(
            generate_class_name("[appHighlight]", AngularDefKind::Directive),
            "AppHighlightDirective"
        );
    }

    #[test]
    fn test_generate_class_name_pipe() {
        assert_eq!(generate_class_name("async", AngularDefKind::Pipe), "AsyncPipe");
        assert_eq!(
            generate_class_name("myCustomPipe", AngularDefKind::Pipe),
            "MyCustomPipe"
        );
    }

    #[test]
    fn test_detect_angular_version() {
        assert_eq!(
            detect_angular_version("g.ɵɵdefineComponent({type:dt})"),
            Some(AngularVersionRange::V10To12)
        );
        assert_eq!(
            detect_angular_version("ɵɵdefer(0, fn)"),
            Some(AngularVersionRange::V17To18)
        );
        assert_eq!(
            detect_angular_version("ɵɵdeclareLet(0)"),
            Some(AngularVersionRange::V19Plus)
        );
        assert_eq!(
            detect_angular_version("angular.module('app', [])"),
            Some(AngularVersionRange::AngularJS)
        );
        assert_eq!(detect_angular_version("console.log('hello')"), None);
    }

    #[test]
    fn test_extract_from_define_component() {
        let source = r#"
            let Fe = ((dt = Le).ɵfac = function(de) {
                return new (de || dt)();
            }, dt.ɵcmp = ɵɵdefineComponent({
                type: dt,
                selectors: [["lms-stepped-progress-indicator"]],
                inputs: {
                    activeStepValue: "activeStepValue",
                    spiData: "spiData"
                },
                decls: 2,
                vars: 1
            }), Le);
        "#;

        let result = extract_angular_metadata(source);
        assert_eq!(result.component_count, 1);
        assert!(!result.definitions.is_empty());

        let comp = &result.definitions[0];
        assert_eq!(comp.selector, "lms-stepped-progress-indicator");
        assert_eq!(comp.class_name, "LmsSteppedProgressIndicatorComponent");
        assert_eq!(comp.inputs, vec!["activeStepValue", "spiData"]);
    }

    #[test]
    fn test_extract_injectable() {
        let source = r#"
            ɵɵdefineInjectable({
                token: dt,
                factory: dt.ɵfac,
                providedIn: "root"
            });
        "#;

        let result = extract_angular_metadata(source);
        assert_eq!(result.injectable_count, 1);
    }

    #[test]
    fn test_extract_pipe() {
        let source = r#"
            ɵɵdefinePipe({
                name: "async",
                type: dt,
                pure: false
            });
        "#;

        let result = extract_angular_metadata(source);
        assert_eq!(result.pipe_count, 1);
        assert_eq!(result.definitions[0].selector, "async");
        assert_eq!(result.definitions[0].class_name, "AsyncPipe");
    }

    #[test]
    fn test_version_detection_v18() {
        // Angular 18 uses ɵɵconditional (new control flow) but not declareLet
        let source = r#"
            ɵɵdefineComponent({selectors: [["app"]]});
            ɵɵconditional(0, ctx.show ? 0 : -1);
        "#;
        assert_eq!(detect_angular_version(source), Some(AngularVersionRange::V17To18));
    }

    #[test]
    fn test_extract_from_webpack_wrapped() {
        let source = r#"
(self.lmsJsonp = self.lmsJsonp || []).push([[8792], {
  5098: (ot, Ee, f) => {
    "use strict";
    var g = f(3953);
    class Le {
      constructor() { this.activeStep = 1; }
    }
    let Fe = ((dt = Le).ɵfac = function(de) {
      return new (de || dt)();
    }, dt.ɵcmp = g.ɵɵdefineComponent({
      type: dt,
      selectors: [["lms-stepped-progress-indicator"]],
      inputs: { activeStepValue: "activeStepValue", spiData: "spiData" },
      decls: 2,
      vars: 1
    }), Le);
  }
}]);
        "#;

        let result = extract_angular_metadata(source);
        assert_eq!(result.component_count, 1);
        assert_eq!(result.definitions[0].selector, "lms-stepped-progress-indicator");
        assert_eq!(result.definitions[0].class_name, "LmsSteppedProgressIndicatorComponent");
    }

    #[test]
    fn test_extract_from_beautified_split_statements() {
        // After beautification, the sequence expression gets split into separate statements
        let source = r#"
(self.lmsJsonp = self.lmsJsonp || []).push([
    [8792],
    {
        5098: (ot, Ee, f) => {
            "use strict";
            var g = f(3953);
            class Le {
                constructor() { this.activeStep = 1; }
            }
            (dt = Le).ɵfac = function(de) {
                return new (de || dt)();
            };
            dt.ɵcmp = g.ɵɵdefineComponent({
                type: dt,
                selectors: [["lms-stepped-progress-indicator"]],
                inputs: { activeStepValue: "activeStepValue", spiData: "spiData" },
                decls: 2,
                vars: 1
            });
        }
    }
]);
        "#;

        let result = extract_angular_metadata(source);
        eprintln!(
            "Split-statement test: components={}, version={:?}",
            result.component_count, result.version
        );
        for def in &result.definitions {
            eprintln!("  Found: {:?} {} ({})", def.kind, def.class_name, def.selector);
        }
        assert_eq!(result.component_count, 1);
        assert_eq!(result.definitions[0].selector, "lms-stepped-progress-indicator");
    }
}
