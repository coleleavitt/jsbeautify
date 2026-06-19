pub(super) const MAX_BODY_STATEMENTS: usize = 5;
pub(super) const MAX_PARAMS: usize = 10;

#[derive(Debug, Clone)]
pub struct InlinableFunction {
    pub name: String,
    pub params: Vec<String>,
    pub return_expression: Option<ReturnExpr>,
    pub is_simple: bool,
}

#[derive(Debug, Clone)]
pub enum ReturnExpr {
    Identifier(String),
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
    Call {
        callee: String,
        args: Vec<Self>,
    },
    ParamRef(usize),
}

#[derive(Debug, Clone, Copy)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    And,
    Or,
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
