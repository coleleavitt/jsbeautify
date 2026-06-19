use oxc_ast::ast::{Argument, Expression};

use super::CommonSubexpressionElimination;

impl CommonSubexpressionElimination {
    pub(super) fn expression_to_string(&self, expr: &Expression<'_>) -> Option<String> {
        match expr {
            Expression::BinaryExpression(bin) => {
                let left = self.expression_to_string(&bin.left)?;
                let op = format!("{:?}", bin.operator);
                let right = self.expression_to_string(&bin.right)?;
                Some(format!("({left} {op} {right})"))
            }
            Expression::CallExpression(call) => {
                let callee = self.expression_to_string(&call.callee)?;
                let args: Vec<String> = call
                    .arguments
                    .iter()
                    .filter_map(|arg| self.argument_to_string(arg))
                    .collect();
                Some(format!("{}({})", callee, args.join(", ")))
            }
            Expression::StaticMemberExpression(static_mem) => {
                let obj = self.expression_to_string(&static_mem.object)?;
                Some(format!("{}.{}", obj, static_mem.property.name))
            }
            Expression::ComputedMemberExpression(computed) => {
                let obj = self.expression_to_string(&computed.object)?;
                let prop = self.expression_to_string(&computed.expression)?;
                Some(format!("{obj}[{prop}]"))
            }
            Expression::Identifier(ident) => Some(ident.name.to_string()),
            Expression::NumericLiteral(lit) => Some(lit.value.to_string()),
            Expression::StringLiteral(lit) => Some(format!("\"{}\"", lit.value)),
            Expression::BooleanLiteral(lit) => Some(lit.value.to_string()),
            Expression::UnaryExpression(unary) => {
                let operand = self.expression_to_string(&unary.argument)?;
                let op = format!("{:?}", unary.operator);
                Some(format!("({op} {operand})"))
            }
            _ => None,
        }
    }

    pub(super) fn argument_to_string(&self, arg: &Argument<'_>) -> Option<String> {
        match arg {
            Argument::Identifier(ident) => Some(ident.name.to_string()),
            Argument::NumericLiteral(lit) => Some(lit.value.to_string()),
            Argument::StringLiteral(lit) => Some(format!("\"{}\"", lit.value)),
            Argument::BooleanLiteral(lit) => Some(lit.value.to_string()),
            Argument::BinaryExpression(bin) => {
                let left = self.expression_to_string(&bin.left)?;
                let op = format!("{:?}", bin.operator);
                let right = self.expression_to_string(&bin.right)?;
                Some(format!("({left} {op} {right})"))
            }
            _ => None,
        }
    }

    pub(super) const fn is_complex_expression(&self, expr: &Expression<'_>) -> bool {
        matches!(
            expr,
            Expression::BinaryExpression(_)
                | Expression::CallExpression(_)
                | Expression::UnaryExpression(_)
                | Expression::ComputedMemberExpression(_)
        )
    }
}
