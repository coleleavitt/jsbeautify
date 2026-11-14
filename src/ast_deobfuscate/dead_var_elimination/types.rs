#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct UnresolvedReference {
    pub(super) name: String,
    pub(super) span_start: u32,
    pub(super) span_end: u32,
}
