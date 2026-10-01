use oxc_ast::ast::{ArrayExpressionElement, Expression, ObjectPropertyKind, Statement};
use rustc_hash::FxHashMap;

use crate::recon_budget::ReconBudget;

use super::types::AngularComponentInfo;

/// Extracts Angular metadata from beautified/deobfuscated JS output.
///
/// Works on the string level (regex + oxc AST) because the patterns are
/// very consistent across Angular compiler versions.
pub struct AngularMetadataExtractor {
    /// All discovered Angular definitions, keyed by minified class name
    pub definitions: Vec<AngularComponentInfo>,
    /// Map from minified name → generated readable name
    pub rename_map: FxHashMap<String, String>,
    pub(super) max_definitions: usize,
}

impl Default for AngularMetadataExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl AngularMetadataExtractor {
    #[must_use]
    pub fn new() -> Self {
        Self::with_limit(usize::MAX)
    }

    #[must_use]
    pub(super) fn with_limit(max_definitions: usize) -> Self {
        Self {
            definitions: Vec::new(),
            rename_map: FxHashMap::default(),
            max_definitions,
        }
    }

    pub(super) fn push_definition(&mut self, info: AngularComponentInfo) {
        if self.definitions.len() < self.max_definitions {
            self.definitions.push(info);
        }
    }

    /// Run extraction on a parsed AST program.
    pub fn extract_from_statements(&mut self, statements: &[Statement<'_>], budget: &mut ReconBudget) {
        for stmt in statements {
            self.visit_statement(stmt, budget);
        }
        self.build_rename_map_with_budget(budget);
    }

    /// Recursively walk a statement looking for Angular define calls at any depth.
    fn visit_statement(&mut self, stmt: &Statement<'_>, budget: &mut ReconBudget) {
        match stmt {
            Statement::VariableDeclaration(var_decl) => {
                for decl in &var_decl.declarations {
                    self.check_variable_declarator(decl, budget);
                    // Recurse into initializer for nested functions/arrows that
                    // might contain OTHER components (webpack module callbacks).
                    // But don't double-process the define call — walk_expression
                    // only recurses into function bodies, not the top-level expr.
                    if let Some(init) = &decl.init {
                        self.walk_into_functions_only(init, budget);
                    }
                }
            }
            Statement::ExpressionStatement(expr_stmt) => {
                self.check_expression_for_define(&expr_stmt.expression, None, budget);
                self.walk_expression(&expr_stmt.expression, budget);
            }
            Statement::BlockStatement(block) => {
                for s in &block.body {
                    self.visit_statement(s, budget);
                }
            }
            Statement::IfStatement(if_stmt) => {
                self.visit_statement(&if_stmt.consequent, budget);
                if let Some(alt) = &if_stmt.alternate {
                    self.visit_statement(alt, budget);
                }
            }
            Statement::ForStatement(for_stmt) => {
                self.visit_statement(&for_stmt.body, budget);
            }
            Statement::WhileStatement(while_stmt) => {
                self.visit_statement(&while_stmt.body, budget);
            }
            Statement::DoWhileStatement(do_while) => {
                self.visit_statement(&do_while.body, budget);
            }
            Statement::ReturnStatement(ret) => {
                if let Some(arg) = &ret.argument {
                    self.walk_expression(arg, budget);
                }
            }
            Statement::TryStatement(try_stmt) => {
                for s in &try_stmt.block.body {
                    self.visit_statement(s, budget);
                }
                if let Some(handler) = &try_stmt.handler {
                    for s in &handler.body.body {
                        self.visit_statement(s, budget);
                    }
                }
                if let Some(finalizer) = &try_stmt.finalizer {
                    for s in &finalizer.body {
                        self.visit_statement(s, budget);
                    }
                }
            }
            Statement::SwitchStatement(switch) => {
                for case in &switch.cases {
                    for s in &case.consequent {
                        self.visit_statement(s, budget);
                    }
                }
            }
            _ => {}
        }
    }

    /// Walk into expressions to find Angular defines inside nested functions/arrows/objects.
    fn walk_expression(&mut self, expr: &Expression<'_>, budget: &mut ReconBudget) {
        match expr {
            Expression::FunctionExpression(func) => {
                if let Some(body) = &func.body {
                    for stmt in &body.statements {
                        self.visit_statement(stmt, budget);
                    }
                }
            }
            Expression::ArrowFunctionExpression(arrow) => {
                if let Some(body) = arrow.body.as_function_body() {
                    for stmt in &body.statements {
                        self.visit_statement(stmt, budget);
                    }
                } else if let Some(expr) = arrow.body.as_expression() {
                    self.walk_expression(expr, budget);
                }
            }
            Expression::CallExpression(call) => {
                // Walk into arguments (webpack callbacks etc.)
                for arg in &call.arguments {
                    if let Some(expr) = arg.as_expression() {
                        self.walk_expression(expr, budget);
                    }
                }
                self.walk_expression(&call.callee, budget);
            }
            Expression::ObjectExpression(obj) => {
                for prop in &obj.properties {
                    if let ObjectPropertyKind::ObjectProperty(p) = prop {
                        self.walk_expression(&p.value, budget);
                    }
                }
            }
            Expression::ArrayExpression(arr) => {
                for elem in &arr.elements {
                    match elem {
                        ArrayExpressionElement::SpreadElement(spread) => {
                            self.walk_expression(&spread.argument, budget);
                        }
                        ArrayExpressionElement::Elision(_) => {}
                        _ => {
                            // All other variants are expression types
                            if let Some(expr) = elem.as_expression() {
                                self.walk_expression(expr, budget);
                            }
                        }
                    }
                }
            }
            Expression::SequenceExpression(seq) => {
                for sub in &seq.expressions {
                    self.check_expression_for_define(sub, None, budget);
                    self.walk_expression(sub, budget);
                }
            }
            Expression::AssignmentExpression(assign) => {
                self.walk_expression(&assign.right, budget);
            }
            Expression::ParenthesizedExpression(paren) => {
                self.walk_expression(&paren.expression, budget);
            }
            Expression::ConditionalExpression(cond) => {
                self.walk_expression(&cond.consequent, budget);
                self.walk_expression(&cond.alternate, budget);
            }
            _ => {}
        }
    }

    /// Like `walk_expression` but only descends into function/arrow bodies.
    /// Used for variable initializers where `check_expression_for_define` already
    /// handles the top-level sequence/assignment, avoiding double-detection.
    fn walk_into_functions_only(&mut self, expr: &Expression<'_>, budget: &mut ReconBudget) {
        match expr {
            Expression::FunctionExpression(func) => {
                if let Some(body) = &func.body {
                    for stmt in &body.statements {
                        self.visit_statement(stmt, budget);
                    }
                }
            }
            Expression::ArrowFunctionExpression(arrow) => {
                if let Some(body) = arrow.body.as_function_body() {
                    for stmt in &body.statements {
                        self.visit_statement(stmt, budget);
                    }
                } else if let Some(expr) = arrow.body.as_expression() {
                    self.walk_into_functions_only(expr, budget);
                }
            }
            Expression::CallExpression(call) => {
                for arg in &call.arguments {
                    if let Some(expr) = arg.as_expression() {
                        self.walk_into_functions_only(expr, budget);
                    }
                }
                self.walk_into_functions_only(&call.callee, budget);
            }
            Expression::ObjectExpression(obj) => {
                for prop in &obj.properties {
                    if let ObjectPropertyKind::ObjectProperty(p) = prop {
                        self.walk_into_functions_only(&p.value, budget);
                    }
                }
            }
            Expression::ArrayExpression(arr) => {
                for elem in &arr.elements {
                    if let Some(expr) = elem.as_expression() {
                        self.walk_into_functions_only(expr, budget);
                    }
                }
            }
            Expression::ParenthesizedExpression(paren) => {
                self.walk_into_functions_only(&paren.expression, budget);
            }
            _ => {}
        }
    }
    pub(super) fn build_rename_map_with_budget(&mut self, budget: &mut ReconBudget) {
        for info in &self.definitions {
            if let Some(export_name) = info.minified_export_name.as_deref()
                && export_name.len() <= 3
                && !info.class_name.is_empty()
            {
                let Some(export_name) = budget.take_str(export_name) else {
                    return;
                };
                let Some(class_name) = budget.take_str(&info.class_name) else {
                    return;
                };
                self.rename_map.insert(export_name, class_name);
            }
            if let Some(minified_class_name) = info.minified_class_name.as_deref()
                && minified_class_name.len() <= 3
                && !info.class_name.is_empty()
            {
                let Some(minified_class_name) = budget.take_str(minified_class_name) else {
                    return;
                };
                let Some(class_name) = budget.take_str(&info.class_name) else {
                    return;
                };
                self.rename_map.insert(minified_class_name, class_name);
            }
        }
    }
}
