use oxc_ast::ast::{ArrayExpressionElement, AssignmentTarget, BinaryExpression, BinaryOperator, Expression};

use crate::ast_deobfuscate::state::OffsetOperation;

pub(super) fn extract_string_array(elements: &[ArrayExpressionElement<'_>]) -> Option<Vec<String>> {
    let mut strings = Vec::with_capacity(elements.len());
    for elem in elements {
        match elem {
            ArrayExpressionElement::StringLiteral(lit) => {
                strings.push(lit.value.to_string());
            }
            _ => return None,
        }
    }
    Some(strings)
}

pub(super) fn extract_offset_assignment(expr: &Expression<'_>, param_name: &str) -> Option<(i32, OffsetOperation)> {
    if let Expression::AssignmentExpression(assign) = expr
        && let AssignmentTarget::AssignmentTargetIdentifier(target) = &assign.left
        && target.name.as_str() == param_name
        && let Expression::BinaryExpression(bin) = &assign.right
    {
        return extract_offset_from_binary(bin, param_name);
    }
    None
}

pub(super) fn extract_offset_from_binary(
    bin: &BinaryExpression<'_>,
    param_name: &str,
) -> Option<(i32, OffsetOperation)> {
    let left_is_param = matches!(&bin.left, Expression::Identifier(id) if id.name.as_str() == param_name);
    if !left_is_param {
        return None;
    }

    let offset = if let Expression::NumericLiteral(lit) = &bin.right {
        lit.value as i32
    } else {
        return None;
    };

    let op = match bin.operator {
        BinaryOperator::Subtraction => OffsetOperation::Subtract,
        BinaryOperator::Addition => OffsetOperation::Add,
        _ => return None,
    };

    Some((offset, op))
}
