use oxc_ast::ast::BinaryOperator;

#[derive(Debug, Clone)]
pub struct OperatorProxyInfo {
    pub operator: BinaryOperator,
}
