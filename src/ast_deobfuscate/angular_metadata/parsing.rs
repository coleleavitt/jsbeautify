use oxc_ast::ast::{
    ArrayExpressionElement, CallExpression, Expression, ObjectPropertyKind, PropertyKey, VariableDeclarator,
};

use super::extractor::AngularMetadataExtractor;
use super::naming::generate_class_name;
use super::types::{AngularComponentInfo, AngularDefKind};

impl AngularMetadataExtractor {
    pub(super) fn check_variable_declarator(&mut self, decl: &VariableDeclarator<'_>) {
        // Pattern: let Ss = ((l = d).ɵfac = ..., l.ɵcmp = ɵɵdefineComponent({...}), d)
        let export_name = decl.id.get_identifier_name().map(|a| a.to_string());

        if let Some(init) = &decl.init {
            self.check_expression_for_define(init, export_name.as_deref());
        }
    }

    pub(super) fn check_expression_for_define(&mut self, expr: &Expression<'_>, export_name: Option<&str>) {
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
