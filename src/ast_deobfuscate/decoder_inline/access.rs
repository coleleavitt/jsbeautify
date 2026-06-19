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
            eprintln!("[AST]       Found computed member expression");

            let array_name = match &member.object {
                Expression::Identifier(ident) => {
                    eprintln!("[AST]         Array: {}", ident.name);
                    ident.name.to_string()
                }
                _ => return None,
            };

            let (offset, offset_op) = match &member.expression {
                Expression::Identifier(ident) if ident.name.as_str() == param_name => {
                    eprintln!("[AST]         Index: {param_name} (no offset)");
                    (0, OffsetOperation::None)
                }
                Expression::BinaryExpression(bin) => {
                    eprintln!("[AST]         Index is binary expression");
                    self.extract_offset_from_binary(bin, param_name)?
                }
                _ => {
                    eprintln!("[AST]         Index is not identifier or binary");
                    return None;
                }
            };

            Some((array_name, offset, offset_op))
        } else {
            eprintln!("[AST]       Return expression is not array access");
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
            eprintln!("[AST]           Left side is not parameter");
            return None;
        }

        let offset = if let Expression::NumericLiteral(lit) = &bin.right {
            eprintln!("[AST]           Right side is number: {}", lit.value);
            lit.value as i32
        } else {
            eprintln!("[AST]           Right side is not numeric literal");
            return None;
        };

        let offset_op = match bin.operator {
            BinaryOperator::Subtraction => {
                eprintln!("[AST]           Operation: subtract");
                OffsetOperation::Subtract
            }
            BinaryOperator::Addition => {
                eprintln!("[AST]           Operation: add");
                OffsetOperation::Add
            }
            _ => {
                eprintln!("[AST]           Operation: unsupported");
                return None;
            }
        };

        Some((offset, offset_op))
    }
}
