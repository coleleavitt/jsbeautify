use oxc_ast::ast::{BinaryOperator, Expression, UnaryOperator};
use rustc_hash::FxHashMap;

pub(super) enum DeferredConst {
    Lit(i64),
    Ident(String),
    Add(Box<Self>, Box<Self>),
    Sub(Box<Self>, Box<Self>),
    Mul(Box<Self>, Box<Self>),
    Array(Option<Box<Self>>),
    ArrayConcat(Box<Self>, Box<Self>),
}

impl DeferredConst {
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
                if bin.operator == BinaryOperator::Addition
                    && let (Some(l_arr), Some(r_arr)) =
                        (Self::from_array_ast(&bin.left), Self::from_array_ast(&bin.right))
                {
                    return Some(Self::ArrayConcat(Box::new(l_arr), Box::new(r_arr)));
                }
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
            Expression::ArrayExpression(arr) => {
                if arr.elements.is_empty() {
                    Some(Self::Array(None))
                } else if arr.elements.len() == 1 {
                    if let Some(e) = arr.elements[0].as_expression() {
                        Some(Self::Array(Some(Box::new(Self::from_ast(e)?))))
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn from_array_ast(expr: &Expression<'_>) -> Option<Self> {
        let Expression::ArrayExpression(arr) = expr else {
            return None;
        };
        if arr.elements.is_empty() {
            Some(Self::Array(None))
        } else if arr.elements.len() == 1 {
            if let Some(e) = arr.elements[0].as_expression() {
                Some(Self::Array(Some(Box::new(Self::from_ast(e)?))))
            } else {
                None
            }
        } else {
            None
        }
    }

    pub(super) fn eval(&self, known: &FxHashMap<String, i64>) -> Option<i64> {
        match self {
            Self::Lit(v) => Some(*v),
            Self::Ident(name) => known.get(name.as_str()).copied(),
            Self::Add(l, r) => l.eval(known)?.checked_add(r.eval(known)?),
            Self::Sub(l, r) => l.eval(known)?.checked_sub(r.eval(known)?),
            Self::Mul(l, r) => l.eval(known)?.checked_mul(r.eval(known)?),
            Self::Array(None) => Some(0),
            Self::Array(Some(inner)) => inner.eval(known),
            Self::ArrayConcat(l, r) => {
                let l_str = l.eval_as_string(known)?;
                let r_str = r.eval_as_string(known)?;
                format!("{l_str}{r_str}").parse::<i64>().ok()
            }
        }
    }

    fn eval_as_string(&self, known: &FxHashMap<String, i64>) -> Option<String> {
        match self {
            Self::Array(None) => Some(String::new()),
            Self::Array(Some(inner)) => inner.eval(known).map(|v| v.to_string()),
            _ => None,
        }
    }
}
