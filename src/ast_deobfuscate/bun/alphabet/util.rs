use rustc_hash::FxHashMap;

use super::charset::{DEFAULT_HEAD, DEFAULT_TAIL};

/// Checks if a character is valid for HEAD (identifier start)
pub(super) const fn is_valid_head_char(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c == '$'
}

/// Checks if a character is valid for TAIL (identifier continuation)
pub(super) const fn is_valid_tail_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$'
}

/// Checks if a name is likely a keyword or common non-minified identifier
pub(super) fn is_likely_keyword_or_common(name: &str) -> bool {
    matches!(
        name,
        // Keywords
        "if" | "in" | "do" | "of" | "is" | "as" | "to" | "or" | "on" | "be" | "at" | "no" | "it" | "an" | "by" |
        // Common abbreviations
        "id" | "fn" | "el" | "fs" | "os" | "db" | "ui" | "io" | "rx" | "tx" | "ok" | "vs" | "re" | "eq" | "ne" |
        "gt" | "lt" | "ge" | "le" | "up" | "md"
    )
}

/// Builds a sorted alphabet string from frequency map
pub(super) fn build_sorted_alphabet(freq: &FxHashMap<char, usize>, include_digits: bool) -> String {
    let mut chars: Vec<(char, usize)> = freq.iter().map(|(&c, &count)| (c, count)).collect();

    // Sort by frequency descending, then by char for stability
    chars.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let mut result: Vec<char> = chars.into_iter().map(|(c, _)| c).collect();

    // Ensure we have all required characters (fill in missing ones from default)
    let default_chars: &str = if include_digits { DEFAULT_TAIL } else { DEFAULT_HEAD };

    for c in default_chars.chars() {
        if !result.contains(&c) {
            result.push(c);
        }
    }

    // Truncate to correct length
    let target_len = if include_digits { 64 } else { 54 };
    result.truncate(target_len);

    result.into_iter().collect()
}
