use rustc_hash::FxHashMap;

/// Default Bun HEAD alphabet (54 chars - valid identifier starts, no digits)
pub const DEFAULT_HEAD: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ_$";

/// Default Bun TAIL alphabet (64 chars - valid identifier continuations, includes digits)
pub const DEFAULT_TAIL: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_$";

/// Represents an extracted Bun minifier alphabet
#[derive(Debug, Clone)]
pub struct BunAlphabet {
    /// HEAD alphabet: 54 characters for first position (no digits)
    pub head: String,
    /// TAIL alphabet: 64 characters for subsequent positions (includes digits)
    pub tail: String,
    /// Reverse mapping: char -> position in HEAD
    head_to_pos: FxHashMap<char, usize>,
    /// Reverse mapping: char -> position in TAIL
    tail_to_pos: FxHashMap<char, usize>,
}

impl BunAlphabet {
    /// Creates a new alphabet from HEAD and TAIL strings
    #[must_use]
    pub fn new(head: String, tail: String) -> Self {
        let head_to_pos: FxHashMap<char, usize> = head.chars().enumerate().map(|(i, c)| (c, i)).collect();
        let tail_to_pos: FxHashMap<char, usize> = tail.chars().enumerate().map(|(i, c)| (c, i)).collect();

        Self {
            head,
            tail,
            head_to_pos,
            tail_to_pos,
        }
    }

    /// Creates the default Bun alphabet (alphabetical order)
    #[must_use]
    pub fn default_alphabet() -> Self {
        Self::new(DEFAULT_HEAD.to_string(), DEFAULT_TAIL.to_string())
    }

    /// Converts a slot number to a minified name using this alphabet
    ///
    /// This is Bun's `numberToMinifiedName` algorithm:
    /// - First char: slot % 54 -> HEAD[index]
    /// - Subsequent chars: (slot / 54) in base-64 -> TAIL[index]
    #[must_use]
    pub fn slot_to_name(&self, mut slot: usize) -> String {
        let head_chars: Vec<char> = self.head.chars().collect();
        let tail_chars: Vec<char> = self.tail.chars().collect();

        let mut name = String::new();

        // First character from HEAD (base-54)
        let first_idx = slot % 54;
        name.push(head_chars.get(first_idx).copied().unwrap_or('?'));
        slot /= 54;

        // Subsequent characters from TAIL (base-64)
        while slot > 0 {
            slot -= 1;
            let idx = slot % 64;
            name.push(tail_chars.get(idx).copied().unwrap_or('?'));
            slot /= 64;
        }

        name
    }

    /// Converts a minified name back to its slot number
    ///
    /// Returns None if the name contains characters not in the alphabet
    #[must_use]
    pub fn name_to_slot(&self, name: &str) -> Option<usize> {
        let chars: Vec<char> = name.chars().collect();
        if chars.is_empty() {
            return None;
        }

        // First character uses HEAD alphabet (base-54)
        let first_pos = self.head_to_pos.get(&chars[0])?;
        let mut slot = *first_pos;

        // Subsequent characters use TAIL alphabet (base-64)
        if chars.len() > 1 {
            let mut multiplier = 54_usize; // First position was base-54
            for &c in &chars[1..] {
                let pos = self.tail_to_pos.get(&c)?;
                slot += (pos + 1) * multiplier;
                multiplier *= 64;
            }
        }

        Some(slot)
    }

    /// Checks if a name looks like a Bun-minified identifier
    #[must_use]
    pub fn is_minified_name(&self, name: &str) -> bool {
        let len = name.len();

        // Must be 1-4 characters (Bun typically uses short names)
        if len == 0 || len > 4 {
            return false;
        }

        // All characters must be in the alphabet
        let chars: Vec<char> = name.chars().collect();

        // First char must be in HEAD
        if !self.head_to_pos.contains_key(&chars[0]) {
            return false;
        }

        // Subsequent chars must be in TAIL
        for &c in &chars[1..] {
            if !self.tail_to_pos.contains_key(&c) {
                return false;
            }
        }

        true
    }
}

impl Default for BunAlphabet {
    fn default() -> Self {
        Self::default_alphabet()
    }
}
