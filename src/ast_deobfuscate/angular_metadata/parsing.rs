use oxc_ast::ast::{
    ArrayExpressionElement, CallExpression, Expression, ObjectPropertyKind, PropertyKey, VariableDeclarator,
};

use super::extractor::AngularMetadataExtractor;
use super::naming::generate_class_name;
use super::types::{AngularComponentInfo, AngularDefKind};
use crate::recon_budget::ReconBudget;

impl AngularMetadataExtractor {
    pub(super) fn check_variable_declarator(&mut self, decl: &VariableDeclarator<'_>, budget: &mut ReconBudget) {
        // Pattern: let Ss = ((l = d).ɵfac = ..., l.ɵcmp = ɵɵdefineComponent({...}), d)
        let export_name = decl.id.get_identifier_name();

        if let Some(init) = &decl.init {
            self.check_expression_for_define(init, export_name.map(|name| name.as_str()), budget);
        }
    }

    pub(super) fn check_expression_for_define(
        &mut self,
        expr: &Expression<'_>,
        export_name: Option<&str>,
        budget: &mut ReconBudget,
    ) {
        match expr {
            Expression::SequenceExpression(seq) => {
                for sub_expr in &seq.expressions {
                    self.check_expression_for_define(sub_expr, export_name, budget);
                }
            }
            Expression::AssignmentExpression(assign) => {
                // l.ɵcmp = ɵɵdefineComponent({...})
                self.check_expression_for_define(&assign.right, export_name, budget);
            }
            Expression::CallExpression(call) => {
                self.check_call_for_define(call, export_name, budget);
            }
            Expression::ParenthesizedExpression(paren) => {
                self.check_expression_for_define(&paren.expression, export_name, budget);
            }
            _ => {}
        }
    }

    fn check_call_for_define(
        &mut self,
        call: &CallExpression<'_>,
        export_name: Option<&str>,
        budget: &mut ReconBudget,
    ) {
        let Some(callee) = self.get_callee_name(call) else {
            return;
        };

        let kind = match callee {
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
                PropertyKey::StaticIdentifier(ident) => ident.name.as_str(),
                PropertyKey::StringLiteral(lit) => lit.value.as_str(),
                _ => continue,
            };

            match key_name {
                "selectors" => {
                    // selectors: [["my-component"]] or selectors: [["", "myDirective", ""]]
                    let Some(value) = self.extract_selector(&prop.value, budget) else {
                        return;
                    };
                    selector = value;
                }
                "inputs" => {
                    let Some(value) = self.extract_string_map_keys(&prop.value, budget) else {
                        return;
                    };
                    inputs = value;
                }
                "outputs" => {
                    let Some(value) = self.extract_string_map_keys(&prop.value, budget) else {
                        return;
                    };
                    outputs = value;
                }
                "name" if kind == AngularDefKind::Pipe => {
                    // Pipe name is in `name: "myPipe"`
                    if let Expression::StringLiteral(lit) = &prop.value {
                        let Some(value) = budget.take_str(lit.value.as_str()) else {
                            return;
                        };
                        pipe_name = value;
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
        if !budget.try_reserve(class_name.len()) {
            return;
        }
        let minified_export_name = match export_name {
            Some(name) => {
                let Some(name) = budget.take_str(name) else {
                    return;
                };
                Some(name)
            }
            None => None,
        };

        let info = AngularComponentInfo {
            selector,
            class_name,
            kind,
            inputs,
            outputs,
            minified_export_name,
            minified_class_name: None,
        };

        self.push_definition(info);
    }

    fn get_callee_name<'a>(&self, call: &'a CallExpression<'_>) -> Option<&'a str> {
        match &call.callee {
            Expression::Identifier(ident) => Some(ident.name.as_str()),
            Expression::StaticMemberExpression(member) => Some(member.property.name.as_str()),
            _ => None,
        }
    }

    /// Extract selector string from `selectors: [["my-comp"]]` or `selectors: [["", "dir", ""]]`
    fn extract_selector(&self, expr: &Expression<'_>, budget: &mut ReconBudget) -> Option<String> {
        let Expression::ArrayExpression(outer) = expr else {
            return Some(String::new());
        };

        // First element is the selector array
        let Some(first) = outer.elements.first() else {
            return Some(String::new());
        };

        let ArrayExpressionElement::ArrayExpression(inner) = first else {
            return Some(String::new());
        };

        // Collect non-empty strings from the inner array
        let mut parts: Vec<String> = Vec::new();
        for elem in &inner.elements {
            if let ArrayExpressionElement::StringLiteral(lit) = elem {
                let value = lit.value.as_str();
                if !value.is_empty() {
                    let value = budget.take_str(value)?;
                    parts.push(value);
                }
            }
        }

        let joined_len = parts.iter().map(String::len).sum();
        if !budget.try_reserve(joined_len) {
            return None;
        }
        Some(parts.join(""))
    }

    /// Extract keys from an inputs/outputs object like `{foo: "foo", bar: "bar"}`
    /// or an array like `["foo", "bar"]`
    fn extract_string_map_keys(&self, expr: &Expression<'_>, budget: &mut ReconBudget) -> Option<Vec<String>> {
        let mut keys = Vec::new();

        match expr {
            Expression::ObjectExpression(obj) => {
                for prop in &obj.properties {
                    if let ObjectPropertyKind::ObjectProperty(p) = prop {
                        match &p.key {
                            PropertyKey::StaticIdentifier(ident) => {
                                let value = budget.take_str(ident.name.as_str())?;
                                keys.push(value);
                            }
                            PropertyKey::StringLiteral(lit) => {
                                let value = budget.take_str(lit.value.as_str())?;
                                keys.push(value);
                            }
                            _ => {}
                        }
                    }
                }
            }
            Expression::ArrayExpression(arr) => {
                for elem in &arr.elements {
                    if let ArrayExpressionElement::StringLiteral(lit) = elem {
                        let value = budget.take_str(lit.value.as_str())?;
                        keys.push(value);
                    }
                }
            }
            _ => {}
        }

        Some(keys)
    }
}
