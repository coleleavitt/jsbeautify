use oxc_ast::ast::{BinaryOperator, Expression, UnaryOperator};
use rustc_hash::FxHashMap;

pub(super) fn try_eval_expr(expr: &Expression<'_>, known: &FxHashMap<String, usize>) -> Option<i64> {
    match expr {
        Expression::NumericLiteral(n) => {
            let v = n.value;
            if v.fract() != 0.0 || v > i64::MAX as f64 || v < i64::MIN as f64 {
                return None;
            }
            Some(v as i64)
        }
        Expression::Identifier(id) => known.get(id.name.as_str()).and_then(|&v| i64::try_from(v).ok()),
        Expression::BinaryExpression(bin) => {
            let l = try_eval_expr(&bin.left, known)?;
            let r = try_eval_expr(&bin.right, known)?;
            match bin.operator {
                BinaryOperator::Addition => l.checked_add(r),
                BinaryOperator::Subtraction => l.checked_sub(r),
                BinaryOperator::Multiplication => l.checked_mul(r),
                _ => None,
            }
        }
        Expression::UnaryExpression(u) if u.operator == UnaryOperator::UnaryPlus => try_eval_expr(&u.argument, known),
        Expression::ParenthesizedExpression(p) => try_eval_expr(&p.expression, known),
        _ => None,
    }
}

pub(super) fn try_eval_as_usize(expr: &Expression<'_>, known: &FxHashMap<String, usize>) -> Option<usize> {
    let v = try_eval_expr(expr, known)?;
    usize::try_from(v).ok()
}

pub(super) enum ConstExpr {
    Lit(i64),
    Ident(String),
    Add(Box<Self>, Box<Self>),
    Sub(Box<Self>, Box<Self>),
    Mul(Box<Self>, Box<Self>),
}

impl ConstExpr {
    pub(super) fn from_ast(expr: &Expression<'_>) -> Option<Self> {
        match expr {
            Expression::NumericLiteral(n) => {
                let v = n.value;
                if v.fract() != 0.0 || v > i64::MAX as f64 || v < i64::MIN as f64 {
                    return None;
                }
                Some(Self::Lit(v as i64))
            }
            Expression::Identifier(id) => Some(Self::Ident(id.name.as_str().to_string())),
            Expression::BinaryExpression(bin) => {
                let l = Box::new(Self::from_ast(&bin.left)?);
                let r = Box::new(Self::from_ast(&bin.right)?);
                match bin.operator {
                    BinaryOperator::Addition => Some(Self::Add(l, r)),
                    BinaryOperator::Subtraction => Some(Self::Sub(l, r)),
                    BinaryOperator::Multiplication => Some(Self::Mul(l, r)),
                    _ => None,
                }
            }
            Expression::UnaryExpression(u) if u.operator == UnaryOperator::UnaryPlus => Self::from_ast(&u.argument),
            Expression::ParenthesizedExpression(p) => Self::from_ast(&p.expression),
            _ => None,
        }
    }

    pub(super) fn eval(&self, known: &FxHashMap<String, usize>) -> Option<i64> {
        match self {
            Self::Lit(v) => Some(*v),
            Self::Ident(name) => known.get(name.as_str()).and_then(|&v| i64::try_from(v).ok()),
            Self::Add(l, r) => l.eval(known)?.checked_add(r.eval(known)?),
            Self::Sub(l, r) => l.eval(known)?.checked_sub(r.eval(known)?),
            Self::Mul(l, r) => l.eval(known)?.checked_mul(r.eval(known)?),
        }
    }
}
