use oxc_ast::ast::{
    Argument, ArrayExpressionElement, BindingPattern, CallExpression, Expression, Statement, VariableDeclaration,
};

use crate::ast_deobfuscate::state::StringArrayInfo;

use super::StringArrayRotation;

impl StringArrayRotation {
    pub(super) fn is_obfuscated_name(name: &str) -> bool {
        name.starts_with("_0x") || name.starts_with("_0X")
    }

    pub(super) fn detect_string_array(&mut self, var_decl: &VariableDeclaration<'_>) -> Option<StringArrayInfo> {
        if var_decl.declarations.len() != 1 {
            return None;
        }

        let decl = &var_decl.declarations[0];

        let var_name = match &decl.id {
            BindingPattern::BindingIdentifier(ident) => ident.name.as_str().to_string(),
            _ => return None,
        };

        if !Self::is_obfuscated_name(&var_name) {
            return None;
        }

        let init = decl.init.as_ref()?;

        let Expression::ArrayExpression(array_expr) = init else {
            return None;
        };

        let mut strings = Vec::new();
        for element in &array_expr.elements {
            if let ArrayExpressionElement::StringLiteral(lit) = element {
                strings.push(lit.value.to_string());
            } else {
                return None;
            }
        }

        if strings.is_empty() {
            return None;
        }

        Some(StringArrayInfo {
            var_name,
            strings,
            rotated: false,
            rotation_count: 0,
        })
    }

    pub(super) fn detect_rotation_iife(&mut self, call_expr: &CallExpression<'_>) -> Option<(String, usize)> {
        deob_trace!("[AST] Checking call for rotation IIFE");

        let func = match &call_expr.callee {
            Expression::FunctionExpression(f) => {
                deob_trace!("[AST]   Direct function");
                f
            }
            Expression::ParenthesizedExpression(paren) => {
                if let Expression::FunctionExpression(f) = &paren.expression {
                    deob_trace!("[AST]   Parenthesized function");
                    f
                } else {
                    deob_trace!("[AST]   Parenthesized but not function");
                    return None;
                }
            }
            _ => {
                deob_trace!("[AST]   Not function or paren");
                return None;
            }
        };

        if func.params.items.is_empty() {
            deob_trace!("[AST]   No params");
            return None;
        }

        let array_arg = call_expr.arguments.first()?;
        let array_name = if let Argument::Identifier(ident) = array_arg {
            deob_trace!("[AST]   Arg: {}", ident.name);
            ident.name.as_str()
        } else {
            deob_trace!("[AST]   Arg not identifier");
            return None;
        };

        if !self.detected_arrays.contains_key(array_name) {
            deob_trace!("[AST]   {array_name} not in arrays");
            return None;
        }

        deob_trace!("[AST]   Analyzing body");
        let rotation_count = self.analyze_rotation_body(&func.body.as_ref()?.statements)?;

        deob_trace!("[AST] Detected rotation IIFE: {array_name} with count {rotation_count}");

        Some((array_name.to_string(), rotation_count))
    }

    fn analyze_rotation_body(&self, statements: &[Statement<'_>]) -> Option<usize> {
        let mut has_rotation_ops = false;
        let mut rotation_count = None;

        deob_trace!("[AST]     Body has {} statements", statements.len());

        for (i, stmt) in statements.iter().enumerate() {
            deob_trace!("[AST]     Scanning statement {i}");
            self.scan_for_rotation(stmt, &mut has_rotation_ops, &mut rotation_count);
        }

        deob_trace!("[AST]     has_rotation_ops: {has_rotation_ops}, rotation_count: {rotation_count:?}");

        if has_rotation_ops && rotation_count.is_some() {
            rotation_count
        } else {
            deob_trace!("[AST]     No rotation detected (ops={has_rotation_ops}, count={rotation_count:?})");
            None
        }
    }

    fn scan_for_rotation(&self, stmt: &Statement<'_>, has_ops: &mut bool, count: &mut Option<usize>) {
        match stmt {
            Statement::VariableDeclaration(var_decl) => {
                deob_trace!(
                    "[AST]       Found VariableDeclaration with {} declarations",
                    var_decl.declarations.len()
                );
                for decl in &var_decl.declarations {
                    if let Some(init) = &decl.init {
                        self.scan_expr_for_rotation(init, has_ops, count);
                    }
                }
            }
            Statement::ExpressionStatement(expr_stmt) => {
                deob_trace!("[AST]       Found ExpressionStatement");
                self.scan_expr_for_rotation(&expr_stmt.expression, has_ops, count);
            }
            Statement::WhileStatement(while_stmt) => {
                deob_trace!("[AST]       Found WhileStatement, scanning body");
                self.scan_for_rotation(&while_stmt.body, has_ops, count);
            }
            Statement::BlockStatement(block) => {
                deob_trace!("[AST]       Found BlockStatement with {} statements", block.body.len());
                for stmt in &block.body {
                    self.scan_for_rotation(stmt, has_ops, count);
                }
            }
            _ => {
                deob_trace!("[AST]       Found other statement type");
            }
        }
    }

    fn scan_expr_for_rotation(&self, expr: &Expression<'_>, has_ops: &mut bool, count: &mut Option<usize>) {
        match expr {
            Expression::CallExpression(call) => {
                deob_trace!("[AST]         Found CallExpression");
                if let Expression::StaticMemberExpression(member) = &call.callee {
                    let method = member.property.name.as_str();
                    deob_trace!("[AST]           Method: {method}");
                    if matches!(method, "push" | "shift" | "unshift" | "pop" | "splice") {
                        deob_trace!("[AST]           ✓ Found rotation operation: {method}");
                        *has_ops = true;
                    }
                }

                for arg in &call.arguments {
                    if let Argument::NumericLiteral(lit) = arg {
                        let num = lit.value as usize;
                        deob_trace!("[AST]           Numeric arg: {num}");
                        if num > 0 && num < 1000 {
                            deob_trace!("[AST]           ✓ Found rotation count: {num}");
                            *count = Some(num);
                        }
                    }
                    if let Argument::Identifier(_ident) = arg
                        && let Expression::StaticMemberExpression(member) = &call.callee
                        && matches!(member.property.name.as_str(), "push" | "shift")
                    {
                        *has_ops = true;
                    }
                }
            }
            Expression::FunctionExpression(func) => {
                deob_trace!("[AST]         Found FunctionExpression, scanning body");
                if let Some(body) = &func.body {
                    for stmt in &body.statements {
                        self.scan_for_rotation(stmt, has_ops, count);
                    }
                }
            }
            _ => {
                deob_trace!("[AST]         Found other expression type");
            }
        }
    }
}
