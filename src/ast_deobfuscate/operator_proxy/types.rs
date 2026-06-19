use oxc_ast::ast::BinaryOperator;

#[derive(Debug, Clone)]
pub struct OperatorProxyInfo {
    pub operator: BinaryOperator,
    pub param1: String,
    pub param2: String,
}
