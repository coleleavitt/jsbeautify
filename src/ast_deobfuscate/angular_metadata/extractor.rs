use oxc_ast::ast::{ArrayExpressionElement, Expression, ObjectPropertyKind, Statement};
use rustc_hash::FxHashMap;

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
                if let Some(body) = arrow.body.as_function_body() {
                    for stmt in &body.statements {
                        self.visit_statement(stmt);
                    }
                } else if let Some(expr) = arrow.body.as_expression() {
                    self.visit_expression(expr);
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

    /// Like `walk_expression` but only descends into function/arrow bodies.
    /// Used for variable initializers where `check_expression_for_define` already
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
                if let Some(body) = arrow.body.as_function_body() {
                    for stmt in &body.statements {
                        self.visit_statement(stmt);
                    }
                } else if let Some(expr) = arrow.body.as_expression() {
                    self.visit_expression(expr);
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
}
