use oxc_syntax::symbol::SymbolId;

pub(super) const MAX_PARAMS: usize = 10;

#[derive(Debug, Clone)]
pub struct InlinableFunction {
    pub name: String,
    pub params: usize,
    pub return_expression: ReturnExpr,
}

#[derive(Debug, Clone)]
pub enum ReturnExpr {
    /// A non-parameter identifier and the binding it resolves to inside the
    /// function (`None` for an unresolved global).
    Identifier {
        name: String,
        symbol: Option<SymbolId>,
    },
    Number(f64),
    String(String),
    Boolean(bool),
    Null,
    Binary {
        left: Box<Self>,
        op: BinaryOp,
        right: Box<Self>,
    },
    Unary {
        op: UnaryOp,
        arg: Box<Self>,
    },
    ParamRef(usize),
}

impl ReturnExpr {
    pub(super) fn uses_param(&self, index: usize) -> bool {
        match self {
            Self::ParamRef(param) => *param == index,
            Self::Binary { left, right, .. } => left.uses_param(index) || right.uses_param(index),
            Self::Unary { arg, .. } => arg.uses_param(index),
            Self::Identifier { .. } | Self::Number(_) | Self::String(_) | Self::Boolean(_) | Self::Null => false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    BitAnd,
    BitOr,
    BitXor,
    Eq,
    NotEq,
    StrictEq,
    StrictNotEq,
    Lt,
    Lte,
    Gt,
    Gte,
}

#[derive(Debug, Clone, Copy)]
pub enum UnaryOp {
    Not,
    Neg,
    Plus,
    BitNot,
}
