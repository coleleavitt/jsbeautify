//! Wordlist generation module.
//!
//! Ports `crates/core/wordlist/` from jxscout-pro-v2.
//!
//! Given JS source, HTML, and/or collected URLs, produces a de-duplicated,
//! noise-filtered word list suitable for use in fuzzing dictionaries,
//! content-discovery, or brute-force attack wordlists.
//!
//! ## Quick start
//!
//! ```rust
//! use js_beautify_rs::wordlist::generate_from_js;
//!
//! let words = generate_from_js("const apiKey = 'prod-secret'; fetch('/api/v2/users');");
//! assert!(words.contains(&"apikey".to_string()) || words.iter().any(|w| w.contains("api")));
//! ```

pub mod extractors;
pub mod filter;
pub mod stopwords;

use rustc_hash::FxHashSet;

/// Generate a wordlist from a JS source string.
#[must_use]
pub fn generate_from_js(source: &str) -> Vec<String> {
    extractors::js_ast_extractor::extract(source)
}

/// Generate a wordlist from raw HTML.
#[must_use]
pub fn generate_from_html(html: &str) -> Vec<String> {
    extractors::html_extractor::extract(html)
}

/// Generate a wordlist from a list of URLs/paths.
#[must_use]
pub fn generate_from_urls(urls: &[&str]) -> Vec<String> {
    extractors::url_path_extractor::extract_from_urls(urls)
}

/// Generate a combined, de-duplicated wordlist from JS source + HTML + URLs.
#[must_use]
pub fn generate_all(js: Option<&str>, html: Option<&str>, urls: &[&str]) -> Vec<String> {
    let mut combined: FxHashSet<String> = FxHashSet::default();

    if let Some(js_src) = js {
        for w in extractors::js_ast_extractor::extract(js_src) {
            combined.insert(w);
        }
    }
    if let Some(html_src) = html {
        for w in extractors::html_extractor::extract(html_src) {
            combined.insert(w);
        }
    }
    for w in extractors::url_path_extractor::extract_from_urls(urls) {
        combined.insert(w);
    }

    let mut out: Vec<String> = combined.into_iter().collect();
    out.sort_unstable();
    out
}
