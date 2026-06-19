#[derive(Debug, Clone)]
pub(super) enum PropertyValue {
    Number(f64),
    String(String),
    Bool(bool),
    Null,
    Identifier(String),
}

#[derive(Debug, Clone)]
pub(super) struct CollectedProperty {
    pub(super) name: String,
    pub(super) value: PropertyValue,
    pub(super) stmt_index: usize,
}
