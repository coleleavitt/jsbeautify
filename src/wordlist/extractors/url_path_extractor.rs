//! Word extraction from URL path segments.
//!
//! Ports `crates/core/wordlist/src/extractors/url_path_extractor.rs`.
//!
//! Given a URL like `https://api.example.com/v2/users/profile?include=avatar`,
//! this extractor splits:
//!   - the hostname into labels (`api`, `example`, `com`)
//!   - the path into segments (`v2`, `users`, `profile`)
//!   - query keys and values (`include`, `avatar`)
//!
//! Everything passes through [`crate::wordlist::filter`].

use rustc_hash::FxHashSet;

use crate::wordlist::filter;

/// Extract words from the URL strings found in `source`.
///
/// `urls` is a slice of raw URL/path strings (as collected by e.g.
/// [`crate::analyzers::path_extractor`]).
#[must_use]
pub fn extract_from_urls(urls: &[&str]) -> Vec<String> {
    let mut words: FxHashSet<String> = FxHashSet::default();
    let mut push = |s: &str| {
        let norm = filter::normalize(s);
        if norm.len() > 1 && !filter::is_noise(&norm) {
            words.insert(norm);
        }
    };

    for raw in urls {
        // Try to parse as a proper URL; fall back to path-split.
        if let Ok(url) = url::Url::parse(raw) {
            // Hostname labels.
            if let Some(host) = url.host_str() {
                for part in host.split('.') {
                    push(part);
                }
            }
            // Path segments.
            for seg in url.path_segments().into_iter().flatten() {
                for token in seg.split(|c| matches!(c, '-' | '_')) {
                    push(token);
                }
            }
            // Query key/values.
            for (k, v) in url.query_pairs() {
                push(&k);
                push(&v);
            }
        } else {
            // Treat as a raw path string.
            for seg in raw.split('/') {
                for token in seg.split(|c| matches!(c, '-' | '_' | '?' | '=')) {
                    push(token);
                }
            }
        }
    }

    let mut out: Vec<String> = words.into_iter().collect();
    out.sort_unstable();
    out
}
