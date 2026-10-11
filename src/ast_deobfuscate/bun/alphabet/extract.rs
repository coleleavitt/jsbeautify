use rustc_hash::FxHashMap;

use super::charset::BunAlphabet;
use super::util::{build_sorted_alphabet, is_likely_keyword_or_common, is_valid_head_char, is_valid_tail_char};

/// Extracts the Bun alphabet from source code by analyzing identifier frequencies
///
/// Algorithm:
/// 1. Count frequency of each single-character identifier -> HEAD ordering
/// 2. Count frequency of second character in 2-char identifiers -> TAIL ordering
#[derive(Debug, Default)]
pub struct AlphabetExtractor {
    /// Frequency count for single-char identifiers (determines HEAD)
    single_char_freq: FxHashMap<char, usize>,
    /// Frequency count for second char in 2-char identifiers (determines TAIL)
    second_char_freq: FxHashMap<char, usize>,
}

impl AlphabetExtractor {
    /// Creates a new alphabet extractor
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records an identifier for frequency analysis
    pub fn record_identifier(&mut self, name: &str) {
        let chars: Vec<char> = name.chars().collect();
        let len = chars.len();

        // Skip non-minified looking names
        if len == 0 || len > 4 {
            return;
        }

        // Skip names that look like keywords or common patterns
        if is_likely_keyword_or_common(name) {
            return;
        }

        // Single-char identifiers contribute to HEAD frequency
        if len == 1 {
            let c = chars[0];
            if is_valid_head_char(c) {
                *self.single_char_freq.entry(c).or_insert(0) += 1;
            }
        }

        // Two-char identifiers: second char contributes to TAIL frequency
        if len == 2 {
            let c = chars[1];
            if is_valid_tail_char(c) {
                *self.second_char_freq.entry(c).or_insert(0) += 1;
            }
        }
    }

    /// Builds the extracted alphabet from recorded frequencies
    #[must_use]
    pub fn build_alphabet(&self) -> BunAlphabet {
        // Build HEAD: sort single-char frequencies descending
        let head = build_sorted_alphabet(&self.single_char_freq, false);

        // Build TAIL: sort second-char frequencies descending
        let tail = build_sorted_alphabet(&self.second_char_freq, true);

        BunAlphabet::new(head, tail)
    }

    /// Returns the number of single-char identifiers recorded
    #[must_use]
    pub fn single_char_count(&self) -> usize {
        self.single_char_freq.values().sum()
    }

    /// Returns the number of two-char identifiers recorded
    #[must_use]
    pub fn two_char_count(&self) -> usize {
        self.second_char_freq.values().sum()
    }
}

/// Extracts the Bun alphabet from raw JavaScript source code
///
/// This is a convenience function that scans the code and extracts all identifiers.
#[must_use]
pub fn extract_alphabet_from_source(source: &str) -> BunAlphabet {
    let mut extractor = AlphabetExtractor::new();

    // Simple regex-free identifier extraction
    // Look for patterns like: var X, let X, const X, function X, (X) =>, X:, X =
    let chars = source.chars();
    let mut current_word = String::new();
    let mut in_string = false;
    let mut string_char = ' ';
    let mut prev = '\0';

    for c in chars {
        // Handle string literals
        if !in_string && (c == '"' || c == '\'' || c == '`') {
            in_string = true;
            string_char = c;
            prev = c;
            continue;
        }
        if in_string {
            if c == string_char && prev != '\\' {
                in_string = false;
            }
            prev = c;
            continue;
        }

        // Build identifier
        if c.is_ascii_alphanumeric() || c == '_' || c == '$' {
            current_word.push(c);
        } else {
            // End of identifier
            if !current_word.is_empty() {
                // Check if it starts with a valid identifier char (not digit)
                if current_word
                    .chars()
                    .next()
                    .is_some_and(|first| first.is_ascii_alphabetic() || first == '_' || first == '$')
                {
                    extractor.record_identifier(&current_word);
                }
                current_word.clear();
            }
        }
        prev = c;
    }

    // Don't forget the last word
    if !current_word.is_empty()
        && current_word
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic() || first == '_' || first == '$')
    {
        extractor.record_identifier(&current_word);
    }

    let alphabet = extractor.build_alphabet();
    eprintln!(
        "[BUN_ALPHABET] Extracted alphabet from {} single-char and {} two-char identifiers",
        extractor.single_char_count(),
        extractor.two_char_count()
    );
    eprintln!("[BUN_ALPHABET] HEAD: {}", &alphabet.head[..20.min(alphabet.head.len())]);
    eprintln!("[BUN_ALPHABET] TAIL: {}", &alphabet.tail[..20.min(alphabet.tail.len())]);

    alphabet
}
