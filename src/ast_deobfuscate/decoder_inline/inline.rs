use oxc_ast::ast::{Argument, CallExpression, Expression, StringLiteral};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use std::cell::Cell;

use crate::ast_deobfuscate::state::OffsetOperation;

use super::{Ctx, DecoderInliner};

impl DecoderInliner {
    pub(super) fn try_inline_decoder_call<'a>(
        &mut self,
        call: &CallExpression<'a>,
        ctx: &mut Ctx<'a>,
    ) -> Option<Expression<'a>> {
        eprintln!("[AST] Checking call for decoder inlining");

        let func_name = if let Expression::Identifier(ident) = &call.callee {
            eprintln!("[AST]   Function: {}", ident.name);
            ident.name.as_str()
        } else {
            eprintln!("[AST]   Callee is not identifier");
            return None;
        };

        let decoder = self.detected_decoders.get(func_name)?;
        eprintln!("[AST]   Found decoder for function {func_name}");

        if call.arguments.len() != 1 {
            eprintln!("[AST]   Call has {} args, need exactly 1", call.arguments.len());
            return None;
        }

        let arg_value = if let Argument::NumericLiteral(lit) = &call.arguments[0] {
            eprintln!("[AST]   Argument: {}", lit.value);
            lit.value as i32
        } else {
            eprintln!("[AST]   Argument is not numeric literal");
            return None;
        };

        eprintln!(
            "[AST]   Decoder offset_operation: {:?}, offset: {}",
            decoder.offset_operation, decoder.offset
        );

        let actual_index = match decoder.offset_operation {
            OffsetOperation::None => {
                eprintln!("[AST]   No offset, using argument value directly: {arg_value}");
                arg_value as usize
            }
            OffsetOperation::Subtract => {
                let result = arg_value - decoder.offset;
                eprintln!(
                    "[AST]   Applying offset: {} - {} = {}",
                    arg_value, decoder.offset, result
                );
                if result < 0 {
                    eprintln!("[AST]   Negative index after offset");
                    return None;
                }
                result as usize
            }
            OffsetOperation::Add => {
                let result = arg_value + decoder.offset;
                eprintln!(
                    "[AST]   Applying offset: {} + {} = {}",
                    arg_value, decoder.offset, result
                );
                result as usize
            }
        };

        let array_info = ctx.state.string_arrays.get(&decoder.array_name)?;
        eprintln!(
            "[AST]   Array {} has {} strings",
            decoder.array_name,
            array_info.strings.len()
        );

        if actual_index >= array_info.strings.len() {
            eprintln!(
                "[AST]   Index {} out of bounds (array has {})",
                actual_index,
                array_info.strings.len()
            );
            return None;
        }

        let raw_value = &array_info.strings[actual_index];
        eprintln!("[AST]   Raw value from array: {raw_value:?}");
        eprintln!("[AST]   Decoder type: {:?}", decoder.decoder_type);

        let decoded_value = self.apply_decoder(raw_value, &decoder.decoder_type)?;

        eprintln!(
            "[AST] ✓ Inlining decoder call: {}({}) → \"{}\" (decoder: {:?})",
            func_name, arg_value, decoded_value, decoder.decoder_type
        );

        Some(Expression::StringLiteral(ctx.ast.alloc(StringLiteral {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            value: ctx.ast.str(&decoded_value),
            raw: None,
            lone_surrogates: false,
        })))
    }
}
