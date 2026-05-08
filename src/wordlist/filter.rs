//! Word-level noise filter.
//!
//! Ports `crates/core/wordlist/src/filter.rs` from jxscout.
//!
//! A word is discarded when it:
//!   - Is empty or only 1 character
//!   - Is a stopword (English, JS keyword, or file extension)
//!   - Looks like a hex digest / base64 blob
//!   - Is short uppercase (CSS/enum constant)
//!   - Starts with a bundler-mangled prefix (`$abc123$`)
//!   - Contains only digits
//!   - Starts with a digit followed by alpha (CSS dimension like `100px`)
//!   - Contains a template placeholder (`${...}`)
//!   - Contains a unicode escape (`\u0041`)

use lazy_static::lazy_static;
use regex::Regex;

use super::stopwords;

lazy_static! {
    static ref HEX: Regex = Regex::new(r"^[0-9a-f]{8,}$").unwrap();
    static ref BASE64: Regex = Regex::new(r"^[A-Za-z0-9+/]{20,}={0,2}$").unwrap();
    static ref SHORT_UPPER: Regex = Regex::new(r"^[A-Z]{2,3}$").unwrap();
    static ref MANGLED: Regex = Regex::new(r"^\$[a-f0-9]+\$").unwrap();
    static ref TMPL: Regex = Regex::new(r"\$\{[^}]*\}").unwrap();
    static ref UNICODE_ESC: Regex = Regex::new(r"[\\][ux][0-9a-fA-F]{2,4}").unwrap();
    static ref NUM_PREFIX: Regex = Regex::new(r"^\d+[a-zA-Z]+").unwrap();
}

/// Return `true` if `word` should be dropped from the wordlist.
#[must_use]
pub fn is_noise(word: &str) -> bool {
    if word.len() <= 1 {
        return true;
    }
    let lower = word.to_ascii_lowercase();
    if is_stopword(&lower) {
        return true;
    }
    if word.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    if HEX.is_match(&lower) {
        return true;
    }
    if BASE64.is_match(word) {
        return true;
    }
    if SHORT_UPPER.is_match(word) {
        return true;
    }
    if MANGLED.is_match(word) {
        return true;
    }
    if TMPL.is_match(word) {
        return true;
    }
    if UNICODE_ESC.is_match(word) {
        return true;
    }
    if NUM_PREFIX.is_match(word) {
        return true;
    }
    false
}

/// Check membership in all three stopword lists (case-insensitive via `lower`).
pub fn is_stopword(lower: &str) -> bool {
    stopwords::ENGLISH.contains(&lower)
        || stopwords::JS_KEYWORDS.contains(&lower)
        || stopwords::EXTENSIONS.contains(&lower)
}

/// Normalize a token: lowercase, strip leading/trailing punctuation.
#[must_use]
pub fn normalize(word: &str) -> String {
    word.trim_matches(|c: char| !c.is_alphanumeric()).to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_is_noise() {
        assert!(is_noise("deadbeefcafe1234"));
    }

    #[test]
    fn stopword_is_noise() {
        assert!(is_noise("the"));
        assert!(is_noise("return"));
    }

    #[test]
    fn short_real_word_passes() {
        assert!(!is_noise("login"));
        assert!(!is_noise("admin"));
        assert!(!is_noise("user"));
    }
}
