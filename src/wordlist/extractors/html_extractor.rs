//! Word extraction from raw HTML source.
//!
//! Ports `crates/core/wordlist/src/extractors/html_extractor.rs`.
//!
//! Pulls words from:
//!   - Visible text nodes (everything not inside a tag)
//!   - `alt`, `title`, `placeholder`, `aria-label` attribute values
//!   - Inline `<script>` block content (as raw token strings)
//!   - `<!-- comment -->` text

use lazy_static::lazy_static;
use regex::Regex;
use rustc_hash::FxHashSet;

use crate::wordlist::filter;

lazy_static! {
    // Strip all HTML tags, leaving text nodes.
    static ref TAG_RE:       Regex = Regex::new(r"<[^>]*>").unwrap();
    // Capture attribute values we care about.
    static ref ATTR_RE:      Regex = Regex::new(
        r#"(?:alt|title|placeholder|aria-label|name)\s*=\s*["']([^"']+)["']"#
    ).unwrap();
    // Pull inline script content.
    static ref SCRIPT_RE:    Regex = Regex::new(
        r"(?si)<script[^>]*>(.*?)</script>"
    ).unwrap();
    // HTML comments.
    static ref COMMENT_RE:   Regex = Regex::new(r"<!--([\s\S]*?)-->").unwrap();
    // Tokenise text into words (alphanum + apostrophe).
    static ref WORD_RE:      Regex = Regex::new(r"[a-zA-Z][a-zA-Z0-9']{1,}").unwrap();
}

/// Extract a de-duplicated, noise-filtered word list from raw HTML.
#[must_use]
pub fn extract(html: &str) -> Vec<String> {
    let mut words: FxHashSet<String> = FxHashSet::default();
    let mut push = |s: &str| {
        let norm = filter::normalize(s);
        if !filter::is_noise(&norm) {
            words.insert(norm);
        }
    };

    // Visible text.
    let stripped = TAG_RE.replace_all(html, " ");
    for m in WORD_RE.find_iter(&stripped) {
        push(m.as_str());
    }

    // Attribute values.
    for cap in ATTR_RE.captures_iter(html) {
        for w in WORD_RE.find_iter(&cap[1]) {
            push(w.as_str());
        }
    }

    // Comment text.
    for cap in COMMENT_RE.captures_iter(html) {
        for w in WORD_RE.find_iter(&cap[1]) {
            push(w.as_str());
        }
    }

    // Inline <script> blocks — tokenise at word level only (no full parse).
    for cap in SCRIPT_RE.captures_iter(html) {
        for w in WORD_RE.find_iter(&cap[1]) {
            push(w.as_str());
        }
    }

    let mut out: Vec<String> = words.into_iter().collect();
    out.sort_unstable();
    out
}
