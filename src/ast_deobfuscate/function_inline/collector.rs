use oxc_ast::ast::{
    BinaryOperator, BindingPattern, CallExpression, Expression, Function, FunctionBody, Statement, UnaryOperator,
};
use oxc_traverse::Traverse;
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;
use super::types::{BinaryOp, InlinableFunction, MAX_BODY_STATEMENTS, MAX_PARAMS, ReturnExpr, UnaryOp};

pub struct FunctionCollector {
    functions: FxHashMap<String, InlinableFunction>,
    call_counts: FxHashMap<String, usize>,
}

impl FunctionCollector {
    #[must_use]
    pub fn new() -> Self {
        Self {
            functions: FxHashMap::default(),
            call_counts: FxHashMap::default(),
        }
    }

    #[must_use]
    pub fn get_single_use_functions(&self) -> FxHashMap<String, InlinableFunction> {
        self.functions
            .iter()
            .filter(|(name, func)| {
                let count = self.call_counts.get(*name).copied().unwrap_or(0);
                count == 1 && func.is_simple
            })
            .map(|(name, func)| (name.clone(), func.clone()))
            .collect()
    }

    fn try_extract_inlinable(&mut self, func: &Function<'_>) -> Option<InlinableFunction> {
        let name = func.id.as_ref()?.name.as_str().to_string();

        if func.r#async || func.generator {
            return None;
        }

        let body = func.body.as_ref()?;

        if body.statements.len() > MAX_BODY_STATEMENTS {
            return None;
        }

        let params: Vec<String> = func
            .params
            .items
            .iter()
            .filter_map(|p| {
                if let BindingPattern::BindingIdentifier(ident) = &p.pattern {
                    Some(ident.name.as_str().to_string())
                } else {
                    None
                }
            })
            .collect();

        if params.len() > MAX_PARAMS || params.len() != func.params.items.len() {
            return None;
        }

        let return_expression = self.extract_return_expression(body, &params);
        let is_simple = return_expression.is_some() && body.statements.len() == 1;

        eprintln!(
            "[AST] Found inlinable function: {} ({} params, simple={})",
            name,
            params.len(),
            is_simple
        );

        Some(InlinableFunction {
            name,
            params,
            return_expression,
            is_simple,
        })
    }

    fn extract_return_expression(&self, body: &FunctionBody<'_>, params: &[String]) -> Option<ReturnExpr> {
        if body.statements.len() != 1 {
            return None;
        }

        if let Statement::ReturnStatement(ret) = &body.statements[0] {
            return ret
                .argument
                .as_ref()
                .and_then(|expr| Self::expr_to_return_expr(expr, params));
        }

        None
    }

    fn expr_to_return_expr(expr: &Expression<'_>, params: &[String]) -> Option<ReturnExpr> {
        match expr {
            Expression::Identifier(ident) => {
                let name = ident.name.as_str();
                params.iter().position(|p| p == name).map_or_else(
                    || Some(ReturnExpr::Identifier(name.to_string())),
                    |idx| Some(ReturnExpr::ParamRef(idx)),
                )
            }
            Expression::NumericLiteral(num) => Some(ReturnExpr::Number(num.value)),
            Expression::StringLiteral(s) => Some(ReturnExpr::String(s.value.as_str().to_string())),
            Expression::BooleanLiteral(b) => Some(ReturnExpr::Boolean(b.value)),
            Expression::NullLiteral(_) => Some(ReturnExpr::Null),
            Expression::BinaryExpression(binary) => {
                let left = Self::expr_to_return_expr(&binary.left, params)?;
                let right = Self::expr_to_return_expr(&binary.right, params)?;
                let op = match binary.operator {
                    BinaryOperator::Addition => BinaryOp::Add,
                    BinaryOperator::Subtraction => BinaryOp::Sub,
                    BinaryOperator::Multiplication => BinaryOp::Mul,
                    BinaryOperator::Division => BinaryOp::Div,
                    BinaryOperator::Remainder => BinaryOp::Mod,
                    BinaryOperator::BitwiseAnd => BinaryOp::BitAnd,
                    BinaryOperator::BitwiseOR => BinaryOp::BitOr,
                    BinaryOperator::BitwiseXOR => BinaryOp::BitXor,
                    BinaryOperator::Equality => BinaryOp::Eq,
                    BinaryOperator::Inequality => BinaryOp::NotEq,
                    BinaryOperator::StrictEquality => BinaryOp::StrictEq,
                    BinaryOperator::StrictInequality => BinaryOp::StrictNotEq,
                    BinaryOperator::LessThan => BinaryOp::Lt,
                    BinaryOperator::LessEqualThan => BinaryOp::Lte,
                    BinaryOperator::GreaterThan => BinaryOp::Gt,
                    BinaryOperator::GreaterEqualThan => BinaryOp::Gte,
                    _ => return None,
                };
                Some(ReturnExpr::Binary {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                })
            }
            Expression::UnaryExpression(unary) => {
                let arg = Self::expr_to_return_expr(&unary.argument, params)?;
                let op = match unary.operator {
                    UnaryOperator::LogicalNot => UnaryOp::Not,
                    UnaryOperator::UnaryNegation => UnaryOp::Neg,
                    UnaryOperator::UnaryPlus => UnaryOp::Plus,
                    UnaryOperator::BitwiseNot => UnaryOp::BitNot,
                    _ => return None,
                };
                Some(ReturnExpr::Unary { op, arg: Box::new(arg) })
            }
            Expression::ParenthesizedExpression(paren) => Self::expr_to_return_expr(&paren.expression, params),
            _ => None,
        }
    }
}

impl Default for FunctionCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for FunctionCollector {
    fn enter_function(&mut self, func: &mut Function<'a>, _ctx: &mut Ctx<'a>) {
        if let Some(inlinable) = self.try_extract_inlinable(func) {
            self.functions.insert(inlinable.name.clone(), inlinable);
        }
    }

    fn enter_call_expression(&mut self, call: &mut CallExpression<'a>, _ctx: &mut Ctx<'a>) {
        if let Expression::Identifier(ident) = &call.callee {
            let name = ident.name.as_str().to_string();
            *self.call_counts.entry(name).or_insert(0) += 1;
        }
    }
}
