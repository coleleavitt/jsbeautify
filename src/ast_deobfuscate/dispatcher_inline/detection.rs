use oxc_ast::ast::{
    ArrowFunctionExpression, BindingPattern, Expression, Function, ObjectPropertyKind, PropertyKey, PropertyKind,
    Statement, VariableDeclaration,
};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::state::{DispatcherInfo, FunctionInfo, ReturnValue};

use super::DispatcherInliner;

impl DispatcherInliner {
    pub(super) fn detect_dispatcher_pattern(&mut self, var_decl: &VariableDeclaration<'_>) -> Option<DispatcherInfo> {
        if var_decl.declarations.len() != 1 {
            return None;
        }

        let decl = &var_decl.declarations[0];

        let BindingPattern::BindingIdentifier(ident) = &decl.id else {
            return None;
        };
        let var_name = ident.name.as_str().to_string();

        let init = decl.init.as_ref()?;

        let Expression::ObjectExpression(obj_expr) = init else {
            return None;
        };

        let mut functions = FxHashMap::default();

        let mut keys = FxHashSet::default();
        for prop in &obj_expr.properties {
            // Every key must be known and unique, so no later property or
            // spread replaces a detected function.
            let ObjectPropertyKind::ObjectProperty(obj_prop) = prop else {
                return None;
            };
            let key = match &obj_prop.key {
                PropertyKey::StringLiteral(s) => s.value.as_str().to_string(),
                PropertyKey::StaticIdentifier(id) => id.name.as_str().to_string(),
                _ => return None,
            };
            if key == "__proto__" || !keys.insert(key.clone()) {
                return None;
            }
            if obj_prop.kind != PropertyKind::Init {
                continue;
            }

            let return_value = match &obj_prop.value {
                Expression::FunctionExpression(func) => Self::extract_function_return(func),
                Expression::ArrowFunctionExpression(arrow) => Self::extract_arrow_return(arrow),
                _ => None,
            };

            if let Some(ret_val) = return_value {
                functions.insert(
                    key.clone(),
                    FunctionInfo {
                        key,
                        return_value: Some(ret_val),
                    },
                );
            }
        }

        if functions.is_empty() {
            return None;
        }

        Some(DispatcherInfo {
            var_name,
            symbol: ident.symbol_id.get(),
            functions,
        })
    }

    fn extract_function_return(func: &Function<'_>) -> Option<ReturnValue> {
        let body = func.body.as_ref()?;
        if body.statements.len() != 1 {
            return None;
        }

        if let Statement::ReturnStatement(ret) = &body.statements[0] {
            ret.argument.as_ref().and_then(Self::extract_literal_value)
        } else {
            None
        }
    }

    fn extract_arrow_return(arrow: &ArrowFunctionExpression<'_>) -> Option<ReturnValue> {
        if let Some(expr) = arrow.body.as_expression() {
            return Self::extract_literal_value(expr);
        }

        let statements = &arrow.body.as_function_body()?.statements;

        if let [Statement::ReturnStatement(ret)] = statements.as_slice() {
            return ret.argument.as_ref().and_then(Self::extract_literal_value);
        }

        None
    }

    pub(super) fn extract_literal_value(expr: &Expression<'_>) -> Option<ReturnValue> {
        match expr {
            Expression::NumericLiteral(lit) => Some(ReturnValue::Number(lit.value)),
            Expression::StringLiteral(lit) => Some(ReturnValue::String(lit.value.to_string())),
            Expression::BooleanLiteral(lit) => Some(ReturnValue::Bool(lit.value)),
            Expression::NullLiteral(_) => Some(ReturnValue::Null),
            _ => None,
        }
    }
}
