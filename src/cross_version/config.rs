#[derive(Debug, Clone)]
pub struct AlignConfig {
    pub sourcemap_path: Option<String>,
    pub align_with: Option<String>,
    pub hash_depth: usize,
    /// Use slot-based naming (sN) instead of hash-based (_rN) for unmapped identifiers
    pub use_slot_naming: bool,
}

impl Default for AlignConfig {
    fn default() -> Self {
        Self {
            sourcemap_path: None,
            align_with: None,
            hash_depth: 12,
            use_slot_naming: true, // Enable by default
        }
    }
}
