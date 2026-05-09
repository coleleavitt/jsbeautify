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
        r#"(?i)<meta[^>]+(?:name|property)\s*=\s*["'](?:og:title|og:description|twitter:description|keywords)["'][^>]+content\s*=\s*["']([^"']+)["']"#
    ).unwrap();
    static ref META_CONTENT_REV_RE: Regex = Regex::new(
        r#"(?i)<meta[^>]+content\s*=\s*["']([^"']+)["'][^>]+(?:name|property)\s*=\s*["'](?:og:title|og:description|twitter:description|keywords)["']"#
    ).unwrap();
    static ref SCRIPT_RE: Regex = Regex::new(r"(?si)<script[^>]*>.*?</script>").unwrap();
    static ref STYLE_RE: Regex = Regex::new(r"(?si)<style[^>]*>.*?</style>").unwrap();
    // Match jxscout's WORD_REGEX exactly: identifier-like tokens with hyphen/underscore preserved.
    // The previous version used [a-zA-Z][a-zA-Z0-9']{1,} which split kebab-case.
    static ref WORD_RE: Regex = Regex::new(r"\b[a-zA-Z][a-zA-Z0-9_\-]{1,}\b").unwrap();

    static ref TITLE_RE: Regex = Regex::new(r"(?si)<title[^>]*>([\s\S]*?)</title>").unwrap();
    static ref META_DESCRIPTION_RE: Regex = Regex::new(
        r#"(?i)<meta[^>]+name\s*=\s*["']description["'][^>]+content\s*=\s*["']([^"']+)["']"#
    ).unwrap();
    static ref META_DESCRIPTION_REV_RE: Regex = Regex::new(
        r#"(?i)<meta[^>]+content\s*=\s*["']([^"']+)["'][^>]+name\s*=\s*["']description["']"#
    ).unwrap();
    // <article data-topic="..."> / <section data-summary="...">
    static ref TOPIC_ATTR_RE: Regex = Regex::new(
        r#"(?i)\b(?:data-)?topic\s*=\s*["']([^"']+)["']"#
    ).unwrap();
    static ref SUMMARY_ATTR_RE: Regex = Regex::new(
        r#"(?i)\b(?:data-)?summary\s*=\s*["']([^"']+)["']"#
    ).unwrap();
    // href="...?foo=bar&baz=qux" — strip the query and pull keys+values
    static ref HREF_RE: Regex = Regex::new(r#"(?i)\bhref\s*=\s*["']([^"']+)["']"#).unwrap();
    static ref ACTION_RE: Regex = Regex::new(r#"(?i)\baction\s*=\s*["']([^"']+)["']"#).unwrap();
}

pub const SOURCE_BODY_TEXT: &str = "html_body_text";
pub const SOURCE_COMMENT: &str = "html_comment";
pub const SOURCE_IMG_ALT: &str = "html_img_alt";
pub const SOURCE_META_CONTENT: &str = "html_meta_content";
pub const SOURCE_TITLE: &str = "html_title";
pub const SOURCE_DESCRIPTION: &str = "html_description";
pub const SOURCE_TOPIC: &str = "html_topic";
pub const SOURCE_SUMMARY: &str = "html_summary";
pub const SOURCE_QUERY_PARAM: &str = "html_query_param";

/// Which HTML extraction sources to enable.
#[derive(Debug, Clone)]
pub struct HtmlExtractConfig {
    pub body_text: bool,
    pub comments: bool,
    pub img_alt: bool,
    pub meta_content: bool,
    pub title: bool,
    pub description: bool,
    pub topic: bool,
    pub summary: bool,
    pub query_param: bool,
}

impl Default for HtmlExtractConfig {
    fn default() -> Self {
        Self {
            body_text: true,
            comments: true,
            img_alt: true,
            meta_content: true,
            title: true,
            description: true,
            topic: true,
            summary: true,
            query_param: true,
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

    // 5. <title>
    if config.title {
        for cap in TITLE_RE.captures_iter(html) {
            for w in WORD_RE.find_iter(&cap[1]) {
                push(w.as_str(), SOURCE_TITLE);
            }
        }
    }

    // 6. <meta name="description">
    if config.description {
        for cap in META_DESCRIPTION_RE.captures_iter(html) {
            for w in WORD_RE.find_iter(&cap[1]) {
                push(w.as_str(), SOURCE_DESCRIPTION);
            }
        }
        for cap in META_DESCRIPTION_REV_RE.captures_iter(html) {
            for w in WORD_RE.find_iter(&cap[1]) {
                push(w.as_str(), SOURCE_DESCRIPTION);
            }
        }
    }

    // 7. topic / data-topic attributes
    if config.topic {
        for cap in TOPIC_ATTR_RE.captures_iter(html) {
            for w in WORD_RE.find_iter(&cap[1]) {
                push(w.as_str(), SOURCE_TOPIC);
            }
        }
    }

    // 8. summary / data-summary attributes
    if config.summary {
        for cap in SUMMARY_ATTR_RE.captures_iter(html) {
            for w in WORD_RE.find_iter(&cap[1]) {
                push(w.as_str(), SOURCE_SUMMARY);
            }
        }
    }

    // 9. URL query params from href= and action= attributes
    if config.query_param {
        for re in [&*HREF_RE, &*ACTION_RE] {
            for cap in re.captures_iter(html) {
                let url = &cap[1];
                if let Some(q_idx) = url.find('?') {
                    let q = &url[q_idx + 1..];
                    // q is `key=val&key2=val2`
                    for piece in q.split('&') {
                        let kv: Vec<&str> = piece.splitn(2, '=').collect();
                        for token in &kv {
                            for w in WORD_RE.find_iter(token) {
                                push(w.as_str(), SOURCE_QUERY_PARAM);
                            }
                        }
                    }
                }
            }
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
        let descr = by_source(SOURCE_DESCRIPTION);
        // jxscout splits "description" into html_description vs the rest into html_meta_content;
        // either source should surface words from the description meta tag.
        assert!(
            !descr.is_empty() || !meta.is_empty(),
            "expected description/meta words; got descr={descr:?}, meta={meta:?}"
        );
    }

    #[test]
    fn extracts_title_description_topic_summary() {
        let html = r#"
            <html>
            <head>
                <title>Admin Console — Internal</title>
                <meta name="description" content="Manage user roles and permissions">
            </head>
            <body>
                <article data-topic="authentication" data-summary="OAuth handshake details">
                </article>
                <a href="/api?action=delete&user_id=42">drop</a>
                <form action="/submit?token=xyz">x</form>
            </body>
            </html>
        "#;
        let words = extract_sourced(html, &HtmlExtractConfig::default());
        let by = |s: &str| -> Vec<String> { words.iter().filter(|w| w.source == s).map(|w| w.word.clone()).collect() };

        let title = by(SOURCE_TITLE);
        assert!(title.iter().any(|w| w == "admin" || w == "console"));

        let descr = by(SOURCE_DESCRIPTION);
        assert!(descr.iter().any(|w| w == "manage" || w == "user" || w == "roles"));

        let topic = by(SOURCE_TOPIC);
        assert!(topic.iter().any(|w| w == "authentication"));

        let summary = by(SOURCE_SUMMARY);
        assert!(summary.iter().any(|w| w == "oauth" || w == "handshake"));

        let qp = by(SOURCE_QUERY_PARAM);
        assert!(
            qp.iter()
                .any(|w| w == "action" || w == "user_id" || w == "delete" || w == "token")
        );
    }

    #[test]
    fn word_regex_preserves_kebab_and_snake_case() {
        let html = "<title>auth-token-key example_value</title>";
        let words = extract_sourced(html, &HtmlExtractConfig::default());
        let title: Vec<&str> = words
            .iter()
            .filter(|w| w.source == SOURCE_TITLE)
            .map(|w| w.word.as_str())
            .collect();
        // jxscout's WORD_REGEX preserves these as one token each.
        assert!(title.iter().any(|w| *w == "auth-token-key"));
        assert!(title.iter().any(|w| *w == "example_value"));
    }
}
