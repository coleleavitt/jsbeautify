#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct UnresolvedReference {
    pub(super) name: String,
    pub(super) span_start: u32,
    pub(super) span_end: u32,
}

#[derive(Debug, Clone)]
pub(super) struct VarInfo {
    pub(super) declaration_count: usize,
    pub(super) read_count: usize,
    pub(super) write_count: usize,
    pub(super) is_exported: bool,
}

impl VarInfo {
    pub(super) const fn new() -> Self {
        Self {
            declaration_count: 0,
            read_count: 0,
            write_count: 0,
            is_exported: false,
        }
    }

    pub(super) const fn is_dead(&self) -> bool {
        self.declaration_count > 0 && self.read_count == 0 && self.write_count == 0 && !self.is_exported
    }
}
