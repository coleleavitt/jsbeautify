use oxc_ast::ast::{Argument, CallExpression, Expression, Str};
use oxc_span::SPAN;

use crate::ast_deobfuscate::state::OffsetOperation;

use super::{Ctx, DecoderInliner};

impl DecoderInliner {
    pub(super) fn try_inline_decoder_call<'a>(
        &mut self,
        call: &CallExpression<'a>,
        ctx: &mut Ctx<'a>,
    ) -> Option<Expression<'a>> {
        deob_trace!("[AST] Checking call for decoder inlining");

        let func_name = if let Expression::Identifier(ident) = &call.callee {
            deob_trace!("[AST]   Function: {}", ident.name);
            ident.name.as_str()
        } else {
            deob_trace!("[AST]   Callee is not identifier");
            return None;
        };

        let decoder = self.detected_decoders.get(func_name)?;
        deob_trace!("[AST]   Found decoder for function {func_name}");

        if call.arguments.len() != 1 {
            deob_trace!("[AST]   Call has {} args, need exactly 1", call.arguments.len());
            return None;
        }

        let arg_value = if let Argument::NumericLiteral(lit) = &call.arguments[0] {
            deob_trace!("[AST]   Argument: {}", lit.value);
            lit.value as i32
        } else {
            deob_trace!("[AST]   Argument is not numeric literal");
            return None;
        };

        deob_trace!(
            "[AST]   Decoder offset_operation: {:?}, offset: {}",
            decoder.offset_operation,
            decoder.offset
        );

        let actual_index = match decoder.offset_operation {
            OffsetOperation::None => {
                deob_trace!("[AST]   No offset, using argument value directly: {arg_value}");
                arg_value as usize
            }
            OffsetOperation::Subtract => {
                let result = arg_value - decoder.offset;
                deob_trace!(
                    "[AST]   Applying offset: {} - {} = {}",
                    arg_value,
                    decoder.offset,
                    result
                );
                if result < 0 {
                    deob_trace!("[AST]   Negative index after offset");
                    return None;
                }
                result as usize
            }
            OffsetOperation::Add => {
                let result = arg_value + decoder.offset;
                deob_trace!(
                    "[AST]   Applying offset: {} + {} = {}",
                    arg_value,
                    decoder.offset,
                    result
                );
                if result < 0 {
                    deob_trace!("[AST]   Negative index after offset");
                    return None;
                }
                result as usize
            }
        };

        let array_info = ctx.state.string_arrays.get(&decoder.array_name)?;
        deob_trace!(
            "[AST]   Array {} has {} strings",
            decoder.array_name,
            array_info.strings.len()
        );

        if actual_index >= array_info.strings.len() {
            deob_trace!(
                "[AST]   Index {} out of bounds (array has {})",
                actual_index,
                array_info.strings.len()
            );
            return None;
        }

        let raw_value = &array_info.strings[actual_index];
        deob_trace!("[AST]   Raw value from array: {raw_value:?}");
        deob_trace!("[AST]   Decoder type: {:?}", decoder.decoder_type);

        let decoded_value = self.apply_decoder(raw_value, &decoder.decoder_type)?;

        deob_trace!(
            "[AST] ✓ Inlining decoder call: {}({}) → \"{}\" (decoder: {:?})",
            func_name,
            arg_value,
            decoded_value,
            decoder.decoder_type
        );

        Some(Expression::new_string_literal(
            SPAN,
            Str::from_str_in(&decoded_value, ctx),
            None,
            ctx,
        ))
    }
}
