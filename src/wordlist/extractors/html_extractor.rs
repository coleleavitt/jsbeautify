//! Word extraction from raw HTML source with named source categorization.
//!
//! Ports `crates/core/wordlist/src/extractors/html_extractor.rs` from jxscout.
//!
//! Extracts words from 4 distinct source categories:
//!   - `html_body_text`     — visible text content (outside tags)
//!   - `html_comment`       — `<!-- ... -->` content
//!   - `html_img_alt`       — `<img alt="...">` attribute values
//!   - `html_meta_content`  — `<meta content="...">` for og:title, description, etc.

use lazy_static::lazy_static;
use regex::Regex;
use rustc_hash::FxHashSet;

use crate::wordlist::filter;

lazy_static! {
    static ref TAG_RE: Regex = Regex::new(r"<[^>]*>").unwrap();
    static ref COMMENT_RE: Regex = Regex::new(r"<!--([\s\S]*?)-->").unwrap();
    static ref IMG_ALT_RE: Regex = Regex::new(r#"(?i)<img[^>]+alt\s*=\s*["']([^"']+)["']"#).unwrap();
    static ref META_CONTENT_RE: Regex = Regex::new(
        r#"(?i)<meta[^>]+(?:name|property)\s*=\s*["'](?:description|og:title|og:description|twitter:description|keywords)["'][^>]+content\s*=\s*["']([^"']+)["']"#
    ).unwrap();
    static ref META_CONTENT_REV_RE: Regex = Regex::new(
        r#"(?i)<meta[^>]+content\s*=\s*["']([^"']+)["'][^>]+(?:name|property)\s*=\s*["'](?:description|og:title|og:description|twitter:description|keywords)["']"#
    ).unwrap();
    static ref SCRIPT_RE: Regex = Regex::new(r"(?si)<script[^>]*>.*?</script>").unwrap();
    static ref STYLE_RE: Regex = Regex::new(r"(?si)<style[^>]*>.*?</style>").unwrap();
    static ref WORD_RE: Regex = Regex::new(r"[a-zA-Z][a-zA-Z0-9']{1,}").unwrap();
}

pub const SOURCE_BODY_TEXT: &str = "html_body_text";
pub const SOURCE_COMMENT: &str = "html_comment";
pub const SOURCE_IMG_ALT: &str = "html_img_alt";
pub const SOURCE_META_CONTENT: &str = "html_meta_content";

/// Which HTML extraction sources to enable.
#[derive(Debug, Clone)]
pub struct HtmlExtractConfig {
    pub body_text: bool,
    pub comments: bool,
    pub img_alt: bool,
    pub meta_content: bool,
}

impl Default for HtmlExtractConfig {
    fn default() -> Self {
        Self {
            body_text: true,
            comments: true,
            img_alt: true,
            meta_content: true,
        }
    }
}

/// A word with its source category.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HtmlSourcedWord {
    pub word: String,
    pub source: &'static str,
}

/// Extract a flat de-duplicated word list (legacy API, all sources).
#[must_use]
pub fn extract(html: &str) -> Vec<String> {
    extract_sourced(html, &HtmlExtractConfig::default())
        .into_iter()
        .map(|w| w.word)
        .collect()
}

/// Extract words with source categorization.
#[must_use]
pub fn extract_sourced(html: &str, config: &HtmlExtractConfig) -> Vec<HtmlSourcedWord> {
    let mut words: FxHashSet<HtmlSourcedWord> = FxHashSet::default();

    let mut push = |s: &str, source: &'static str| {
        let norm = filter::normalize(s);
        if !filter::is_noise(&norm) {
            words.insert(HtmlSourcedWord { word: norm, source });
        }
    };

    // 1. HTML comments
    if config.comments {
        for cap in COMMENT_RE.captures_iter(html) {
            for w in WORD_RE.find_iter(&cap[1]) {
                push(w.as_str(), SOURCE_COMMENT);
            }
        }
    }

    // 2. img alt attributes
    if config.img_alt {
        for cap in IMG_ALT_RE.captures_iter(html) {
            for w in WORD_RE.find_iter(&cap[1]) {
                push(w.as_str(), SOURCE_IMG_ALT);
            }
        }
    }

    // 3. meta content (description, og:title, etc.)
    if config.meta_content {
        for cap in META_CONTENT_RE.captures_iter(html) {
            for w in WORD_RE.find_iter(&cap[1]) {
                push(w.as_str(), SOURCE_META_CONTENT);
            }
        }
        for cap in META_CONTENT_REV_RE.captures_iter(html) {
            for w in WORD_RE.find_iter(&cap[1]) {
                push(w.as_str(), SOURCE_META_CONTENT);
            }
        }
    }

    // 4. Body text (strip scripts, styles, tags)
    if config.body_text {
        let no_scripts = SCRIPT_RE.replace_all(html, " ");
        let no_styles = STYLE_RE.replace_all(&no_scripts, " ");
        let stripped = TAG_RE.replace_all(&no_styles, " ");
        for w in WORD_RE.find_iter(&stripped) {
            push(w.as_str(), SOURCE_BODY_TEXT);
        }
    }

    let mut out: Vec<HtmlSourcedWord> = words.into_iter().collect();
    out.sort_unstable_by(|a, b| a.word.cmp(&b.word));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_from_all_sources() {
        let html = r#"
            <html>
            <head>
                <meta name="description" content="Secure admin portal for internal use">
            </head>
            <body>
                <!-- TODO: add authentication check here -->
                <img src="logo.png" alt="Company Dashboard Logo">
                <p>Welcome to the management panel</p>
            </body>
            </html>
        "#;
        let words = extract_sourced(html, &HtmlExtractConfig::default());
        let by_source =
            |s: &str| -> Vec<String> { words.iter().filter(|w| w.source == s).map(|w| w.word.clone()).collect() };

        let body = by_source(SOURCE_BODY_TEXT);
        assert!(body.iter().any(|w| w == "welcome" || w == "management" || w == "panel"));

        let comments = by_source(SOURCE_COMMENT);
        assert!(comments.iter().any(|w| w == "authentication" || w == "todo"));

        let alts = by_source(SOURCE_IMG_ALT);
        assert!(alts.iter().any(|w| w == "company" || w == "dashboard" || w == "logo"));

        let meta = by_source(SOURCE_META_CONTENT);
        assert!(
            meta.iter()
                .any(|w| w == "secure" || w == "admin" || w == "portal" || w == "internal")
        );
    }
}
