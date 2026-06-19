//! Bun minifier alphabet extraction and slot computation
//!
//! Bun's minifier uses a frequency-based alphabet ordering:
//! - HEAD (54 chars): Valid identifier start chars (a-z, A-Z, _, $) ordered by frequency
//! - TAIL (64 chars): Valid identifier continuation chars (a-z, A-Z, 0-9, _, $) ordered by frequency
//!
//! This module extracts the alphabet from a minified bundle by analyzing character
//! frequencies, then provides slot number computation for any minified name.
//!
//! The key insight: slot numbers are stable across versions because they represent
//! the frequency rank of each symbol. By normalizing names to slots, we get
//! alphabet-independent comparisons.

mod alphabet;
mod extract;
mod util;

pub use alphabet::{BunAlphabet, DEFAULT_HEAD, DEFAULT_TAIL};
pub use extract::{AlphabetExtractor, extract_alphabet_from_source};

#[cfg(test)]
mod tests;
