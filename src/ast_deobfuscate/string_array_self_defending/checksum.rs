use oxc_ast::ast::{BinaryOperator, Expression, Statement, UnaryOperator};

use super::types::ChecksumTerm;

pub(super) fn extract_checksum_terms(stmts: &[Statement<'_>]) -> Vec<ChecksumTerm> {
    for stmt in stmts {
        match stmt {
            Statement::WhileStatement(w) => {
                let result = extract_checksum_terms_from_stmt(&w.body);
                if !result.is_empty() {
                    return result;
                }
            }
            Statement::BlockStatement(b) => {
                let result = extract_checksum_terms(&b.body);
                if !result.is_empty() {
                    return result;
                }
            }
            _ => {
                let result = extract_checksum_terms_from_stmt(stmt);
                if !result.is_empty() {
                    return result;
                }
            }
        }
    }
    Vec::new()
}

pub(super) fn extract_checksum_terms_from_stmt(stmt: &Statement<'_>) -> Vec<ChecksumTerm> {
    match stmt {
        Statement::TryStatement(t) => {
            for s in &t.block.body {
                let result = extract_checksum_terms_from_stmt(s);
                if !result.is_empty() {
                    return result;
                }
            }
            Vec::new()
        }
        Statement::VariableDeclaration(v) => {
            for decl in &v.declarations {
                if let Some(init) = &decl.init {
                    let mut terms = Vec::new();
                    collect_additive_terms(init, 1, &mut terms);
                    if !terms.is_empty() {
                        return terms;
                    }
                }
            }
            Vec::new()
        }
        Statement::BlockStatement(b) => extract_checksum_terms(&b.body),
        _ => Vec::new(),
    }
}

fn collect_additive_terms(expr: &Expression<'_>, sign: i8, terms: &mut Vec<ChecksumTerm>) {
    match expr {
        Expression::BinaryExpression(bin) => match bin.operator {
            BinaryOperator::Addition => {
                collect_additive_terms(&bin.left, sign, terms);
                collect_additive_terms(&bin.right, sign, terms);
            }
            BinaryOperator::Subtraction => {
                collect_additive_terms(&bin.left, sign, terms);
                collect_additive_terms(&bin.right, -sign, terms);
            }
            BinaryOperator::Division => {
                let (inner_sign, inner_expr) = strip_unary_neg(&bin.left);
                if let Some(idx) = extract_parseint_decoder_idx(inner_expr)
                    && let Expression::NumericLiteral(lit) = &bin.right
                {
                    terms.push(ChecksumTerm {
                        indices: vec![idx],
                        divisors: vec![lit.value],
                        sign: sign * inner_sign,
                    });
                }
            }
            BinaryOperator::Multiplication => {
                let left_term = extract_single_term(&bin.left, 1);
                let right_term = extract_single_term(&bin.right, 1);
                if let (Some(lt), Some(rt)) = (left_term, right_term) {
                    let mut indices = lt.indices;
                    indices.extend(rt.indices);
                    let mut divisors = lt.divisors;
                    divisors.extend(rt.divisors);
                    let combined_sign = sign * lt.sign * rt.sign;
                    terms.push(ChecksumTerm {
                        indices,
                        divisors,
                        sign: combined_sign,
                    });
                }
            }
            _ => {}
        },
        Expression::UnaryExpression(un) if un.operator == UnaryOperator::UnaryNegation => {
            collect_additive_terms(&un.argument, -sign, terms);
        }
        Expression::ParenthesizedExpression(p) => {
            collect_additive_terms(&p.expression, sign, terms);
        }
        _ => {
            if let Some(term) = extract_single_term(expr, sign) {
                terms.push(term);
            }
        }
    }
}

fn extract_single_term(expr: &Expression<'_>, sign: i8) -> Option<ChecksumTerm> {
    match expr {
        Expression::BinaryExpression(bin) if bin.operator == BinaryOperator::Division => {
            // Handle: parseInt(d(IDX)) / DIVISOR  or  -parseInt(d(IDX)) / DIVISOR
            let (inner_sign, inner_expr) = strip_unary_neg(&bin.left);
            let idx = extract_parseint_decoder_idx(inner_expr)?;
            if let Expression::NumericLiteral(lit) = &bin.right {
                Some(ChecksumTerm {
                    indices: vec![idx],
                    divisors: vec![lit.value],
                    sign: sign * inner_sign,
                })
            } else {
                None
            }
        }
        Expression::UnaryExpression(un) if un.operator == UnaryOperator::UnaryNegation => {
            extract_single_term(&un.argument, -sign)
        }
        Expression::ParenthesizedExpression(p) => extract_single_term(&p.expression, sign),
        _ => None,
    }
}

fn strip_unary_neg<'a>(expr: &'a Expression<'a>) -> (i8, &'a Expression<'a>) {
    match expr {
        Expression::UnaryExpression(un) if un.operator == UnaryOperator::UnaryNegation => {
            let (inner_sign, inner) = strip_unary_neg(&un.argument);
            (-inner_sign, inner)
        }
        Expression::ParenthesizedExpression(p) => strip_unary_neg(&p.expression),
        _ => (1, expr),
    }
}

fn extract_parseint_decoder_idx(expr: &Expression<'_>) -> Option<i32> {
    let call = match expr {
        Expression::CallExpression(c) => c,
        Expression::ParenthesizedExpression(p) => {
            if let Expression::CallExpression(c) = &p.expression {
                c
            } else {
                return None;
            }
        }
        _ => return None,
    };

    let is_parseint = match &call.callee {
        Expression::Identifier(id) => id.name.as_str() == "parseInt",
        _ => false,
    };
    if !is_parseint {
        return None;
    }

    let first_arg = call.arguments.first()?;
    if let Some(arg_expr) = first_arg.as_expression()
        && let Expression::CallExpression(inner) = arg_expr
    {
        let idx_arg = inner.arguments.first()?;
        if let Some(idx_expr) = idx_arg.as_expression()
            && let Expression::NumericLiteral(lit) = idx_expr
        {
            return Some(lit.value as i32);
        }
    }
    None
}
