use oxc_ast::ast::{AssignmentTarget, BinaryExpression, BinaryOperator, Expression};

use crate::ast_deobfuscate::state::OffsetOperation;

use super::DecoderInliner;

impl DecoderInliner {
    pub(super) fn extract_simple_array_access(
        &self,
        expr: &Expression<'_>,
        param_name: &str,
    ) -> Option<(String, i32, OffsetOperation)> {
        if let Expression::ComputedMemberExpression(member) = expr {
            deob_trace!("[AST]       Found computed member expression");

            let array_name = match &member.object {
                Expression::Identifier(ident) => {
                    deob_trace!("[AST]         Array: {}", ident.name);
                    ident.name.to_string()
                }
                _ => return None,
            };

            let (offset, offset_op) = match &member.expression {
                Expression::Identifier(ident) if ident.name.as_str() == param_name => {
                    deob_trace!("[AST]         Index: {param_name} (no offset)");
                    (0, OffsetOperation::None)
                }
                Expression::BinaryExpression(bin) => {
                    deob_trace!("[AST]         Index is binary expression");
                    self.extract_offset_from_binary(bin, param_name)?
                }
                _ => {
                    deob_trace!("[AST]         Index is not identifier or binary");
                    return None;
                }
            };

            Some((array_name, offset, offset_op))
        } else {
            deob_trace!("[AST]       Return expression is not array access");
            None
        }
    }

    pub(super) fn extract_offset_assignment(
        &self,
        expr: &Expression<'_>,
        param_name: &str,
    ) -> Option<(i32, OffsetOperation)> {
        if let Expression::AssignmentExpression(assign) = expr
            && let AssignmentTarget::AssignmentTargetIdentifier(target) = &assign.left
            && target.name.as_str() == param_name
            && let Expression::BinaryExpression(bin) = &assign.right
        {
            return self.extract_offset_from_binary(bin, param_name);
        }
        None
    }

    fn extract_offset_from_binary(
        &self,
        bin: &BinaryExpression<'_>,
        param_name: &str,
    ) -> Option<(i32, OffsetOperation)> {
        let left_is_param = matches!(&bin.left, Expression::Identifier(id) if id.name.as_str() == param_name);

        if !left_is_param {
            deob_trace!("[AST]           Left side is not parameter");
            return None;
        }

        let offset = if let Expression::NumericLiteral(lit) = &bin.right {
            deob_trace!("[AST]           Right side is number: {}", lit.value);
            lit.value as i32
        } else {
            deob_trace!("[AST]           Right side is not numeric literal");
            return None;
        };

        let offset_op = match bin.operator {
            BinaryOperator::Subtraction => {
                deob_trace!("[AST]           Operation: subtract");
                OffsetOperation::Subtract
            }
            BinaryOperator::Addition => {
                deob_trace!("[AST]           Operation: add");
                OffsetOperation::Add
            }
            _ => {
                deob_trace!("[AST]           Operation: unsupported");
                return None;
            }
        };

        Some((offset, offset_op))
    }
}
