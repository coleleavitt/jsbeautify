use oxc_ast::ast::{BindingPattern, CallExpression, Expression, Function, Statement};

use crate::ast_deobfuscate::state::{DecoderInfo, DecoderType, OffsetOperation};

use super::{Ctx, DecoderInliner};

impl DecoderInliner {
    pub(super) fn detect_decoder_function<'a>(
        &mut self,
        func: &Function<'a>,
        func_name: &str,
        ctx: &Ctx<'a>,
    ) -> Option<DecoderInfo> {
        deob_trace!("[AST] Analyzing function: {func_name}");

        if func.params.items.len() != 1 {
            deob_trace!(
                "[AST]   Function has {} params, need exactly 1",
                func.params.items.len()
            );
            return None;
        }

        let param_name = if let BindingPattern::BindingIdentifier(ident) = &func.params.items[0].pattern {
            deob_trace!("[AST]   Parameter: {}", ident.name);
            ident.name.as_str()
        } else {
            deob_trace!("[AST]   Parameter is not simple identifier");
            return None;
        };

        let body = func.body.as_ref()?;
        deob_trace!("[AST]   Function body has {} statements", body.statements.len());

        let (array_name, offset, offset_op, decoder_type) =
            self.analyze_function_body(&body.statements, param_name, ctx)?;

        deob_trace!(
            "[AST] ✓ Detected decoder: {func_name} for array {array_name} (offset: {offset_op:?} {offset}, type: {decoder_type:?})"
        );

        Some(DecoderInfo {
            function_name: func_name.to_string(),
            array_name,
            offset,
            offset_operation: offset_op,
            decoder_type,
        })
    }

    fn analyze_function_body<'a>(
        &self,
        statements: &[Statement<'a>],
        param_name: &str,
        ctx: &Ctx<'a>,
    ) -> Option<(String, i32, OffsetOperation, DecoderType)> {
        deob_trace!("[AST]   Analyzing body with {} statements", statements.len());

        let mut array_name = None;
        let mut offset = 0i32;
        let mut offset_op = OffsetOperation::None;
        let mut decoder_type = DecoderType::Simple;

        for stmt in statements {
            match stmt {
                Statement::ReturnStatement(ret) => {
                    deob_trace!("[AST]     Found return statement");
                    if let Some(arg) = &ret.argument
                        && let Some((arr, off, op, dec_type)) =
                            self.extract_array_access_with_decoder(arg, param_name, ctx)
                    {
                        deob_trace!(
                            "[AST]       extract_array_access returned: array={arr}, offset={off}, op={op:?}, decoder={dec_type:?}"
                        );
                        array_name = Some(arr);
                        decoder_type = dec_type;
                        if offset_op == OffsetOperation::None {
                            deob_trace!("[AST]       No assignment offset found, using inline offset");
                            offset = off;
                            offset_op = op;
                        } else {
                            deob_trace!(
                                "[AST]       Already have assignment offset ({offset_op:?} {offset}), keeping it"
                            );
                        }
                    }
                }
                Statement::ExpressionStatement(expr_stmt) => {
                    deob_trace!("[AST]     Found expression statement");
                    if let Some((off, op)) = self.extract_offset_assignment(&expr_stmt.expression, param_name) {
                        offset = off;
                        offset_op = op;
                        deob_trace!("[AST]       Extracted offset assignment: {op:?} {off}");
                    }
                }
                _ => {
                    deob_trace!("[AST]     Found other statement type");
                }
            }
        }

        array_name.map_or_else(
            || {
                deob_trace!("[AST]     No array access found");
                None
            },
            |arr| {
                if ctx.state.string_arrays.contains_key(&arr) {
                    deob_trace!("[AST]     Array {arr} is a known string array");
                    Some((arr, offset, offset_op, decoder_type))
                } else {
                    deob_trace!("[AST]     Array {arr} not found in string arrays");
                    None
                }
            },
        )
    }

    fn extract_array_access_with_decoder<'a>(
        &self,
        expr: &Expression<'a>,
        param_name: &str,
        _ctx: &Ctx<'a>,
    ) -> Option<(String, i32, OffsetOperation, DecoderType)> {
        if let Some(decoder_type) = self.detect_decoder_call(expr) {
            deob_trace!("[AST]       Detected decoder call: {decoder_type:?}");
            if let Expression::CallExpression(call) = expr
                && let Some(arg) = call.arguments.first()
                && let Some(arg_expr) = arg.as_expression()
                && let Some((arr, off, op)) = self.extract_simple_array_access(arg_expr, param_name)
            {
                return Some((arr, off, op, decoder_type));
            }
        }

        if let Some((arr, off, op)) = self.extract_simple_array_access(expr, param_name) {
            return Some((arr, off, op, DecoderType::Simple));
        }

        None
    }

    fn detect_decoder_call(&self, expr: &Expression<'_>) -> Option<DecoderType> {
        if let Expression::CallExpression(call) = expr {
            if let Expression::Identifier(func_id) = &call.callee {
                let func_name = func_id.name.as_str();
                deob_trace!("[AST]         detect_decoder_call: func_name = {func_name}");

                if func_name == "atob" {
                    deob_trace!("[AST]         Detected atob (base64)");
                    return Some(DecoderType::Base64);
                }

                if func_name.contains("xor") || func_name.contains("XOR") {
                    deob_trace!("[AST]         Function contains 'xor', trying to extract key");
                    if let Some(key) = self.extract_xor_key(call) {
                        deob_trace!("[AST]         Extracted XOR key: {key:?}");
                        return Some(DecoderType::Xor { key });
                    }
                    deob_trace!("[AST]         Failed to extract XOR key");
                }

                if func_name.contains("rc4") || func_name.contains("RC4") {
                    deob_trace!("[AST]         Function contains 'rc4', trying to extract key");
                    if let Some(key) = self.extract_rc4_key(call) {
                        deob_trace!("[AST]         Extracted RC4 key: {key:?}");
                        return Some(DecoderType::Rc4 { key });
                    }
                    deob_trace!("[AST]         Failed to extract RC4 key");
                }

                deob_trace!("[AST]         No decoder pattern matched for function: {func_name}");
            } else {
                deob_trace!("[AST]         Callee is not an identifier");
            }
        } else {
            deob_trace!("[AST]         Expression is not a call expression");
        }
        None
    }

    fn extract_xor_key(&self, call: &CallExpression<'_>) -> Option<Vec<u8>> {
        deob_trace!("[AST]           extract_xor_key: args len = {}", call.arguments.len());
        if call.arguments.len() >= 2 {
            if let Some(arg) = call.arguments.get(1) {
                deob_trace!("[AST]           Got second argument");
                if let Some(expr) = arg.as_expression() {
                    deob_trace!("[AST]           Argument is expression");
                    if let Expression::StringLiteral(lit) = expr {
                        let key = lit.value.as_bytes().to_vec();
                        deob_trace!("[AST]           String literal key: {key:?}");
                        return Some(key);
                    }
                    if let Expression::NumericLiteral(lit) = expr {
                        let key = vec![lit.value as u8];
                        deob_trace!("[AST]           Numeric literal key: {key:?}");
                        return Some(key);
                    }
                    deob_trace!("[AST]           Argument is not string or numeric literal");
                } else {
                    deob_trace!("[AST]           Argument is not expression");
                }
            } else {
                deob_trace!("[AST]           No second argument");
            }
        } else {
            deob_trace!("[AST]           Not enough arguments (need at least 2)");
        }
        None
    }

    fn extract_rc4_key(&self, call: &CallExpression<'_>) -> Option<Vec<u8>> {
        if call.arguments.len() >= 2
            && let Some(arg) = call.arguments.get(1)
            && let Some(expr) = arg.as_expression()
            && let Expression::StringLiteral(lit) = expr
        {
            return Some(lit.value.as_bytes().to_vec());
        }
        None
    }
}
