//! HTML `<script>` block extraction.
//!
//! Ports `crates/core/html-js-extractor/src/lib.rs` from jxscout.
//!
//! Pulls inline `<script>` contents from HTML so they can be fed through
//! the analyzer pipeline.

use regex::Regex;
use std::sync::LazyLock;

static SCRIPT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?si)<script(?:\s[^>]*)?>(?P<body>.*?)</script>").unwrap());
static TYPE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?i)type\s*=\s*["']([^"']+)["']"#).unwrap());

/// Allowed MIME types for JavaScript content.
const JS_TYPES: &[&str] = &[
    "text/javascript",
    "application/javascript",
    "application/ecmascript",
    "module",
    "",
];

/// An extracted inline script block.
#[derive(Debug, Clone)]
pub struct ScriptBlock {
    /// The raw JS source content (without `<script>` tags).
    pub content: String,
    /// Byte offset in the original HTML where `<script>` starts.
    pub start_offset: usize,
    /// Whether this is `type="module"`.
    pub is_module: bool,
}

/// Extract all inline `<script>` blocks from `html`.
///
/// Skips `<script src="...">` (external scripts) and non-JS types
/// (e.g. `type="application/json"`, `type="text/template"`).
///
/// # Panics
///
/// Panics only if a compile-time regex pattern is invalid.
#[must_use]
pub fn extract_scripts(html: &str) -> Vec<ScriptBlock> {
    let mut blocks = Vec::new();

    for cap in SCRIPT_RE.captures_iter(html) {
        let full = cap.get(0).unwrap();
        let tag_start = &html[full.start()..];

        // Skip external scripts (have a `src` attribute)
        if tag_start.contains("src=") || tag_start.contains("src =") {
            // Check that src= appears before the closing `>`
            let close_gt = tag_start.find('>').unwrap_or(tag_start.len());
            let src_pos = tag_start.find("src=").or_else(|| tag_start.find("src ="));
            if let Some(sp) = src_pos
                && sp < close_gt
            {
                continue;
            }
        }

        // Check type attribute
        let tag_header = &html[full.start()..=(full.start() + html[full.start()..].find('>').unwrap_or(0))];
        let script_type = TYPE_RE
            .captures(tag_header)
            .map(|c| c[1].to_ascii_lowercase())
            .unwrap_or_default();

        let is_js = JS_TYPES.contains(&script_type.as_str());
        if !is_js {
            continue;
        }

        let body = cap.name("body").map_or("", |m| m.as_str());
        if body.trim().is_empty() {
            continue;
        }

        blocks.push(ScriptBlock {
            content: body.to_string(),
            start_offset: full.start(),
            is_module: script_type == "module",
        });
    }

    blocks
}

/// Extract scripts and run all analyzers on each block.
///
/// Returns a flat list of all matches from all inline scripts.
#[must_use]
pub fn analyze_html(html: &str, file_path: Option<&str>, config: &super::AnalyzerConfig) -> Vec<super::AnalyzerMatch> {
    use super::match_kind::FileType;

    let mut all_matches = Vec::new();
    for block in extract_scripts(html) {
        let matches = super::run_all(&block.content, file_path, FileType::Html, config);
        all_matches.extend(matches);
    }
    all_matches
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_inline_scripts() {
        let html = r#"
            <html>
            <body>
                <script>const apiKey = "ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";</script>
                <script src="external.js"></script>
                <script type="application/json">{"not": "js"}</script>
                <script type="module">import x from "react";</script>
            </body>
            </html>
        "#;
        let blocks = extract_scripts(html);
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].content.contains("apiKey"));
        assert!(blocks[1].content.contains("import"));
        assert!(blocks[1].is_module);
    }

    #[test]
    fn skips_empty_and_external() {
        let html = r#"
            <script src="https://cdn.example.com/app.js"></script>
            <script></script>
            <script>   </script>
        "#;
        let blocks = extract_scripts(html);
        assert!(blocks.is_empty());
    }
}
