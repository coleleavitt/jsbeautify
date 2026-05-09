//! Asset-relationship graph for HTML pages.
//!
//! Pure-data port of jxscout's `project-db/js_loader_relationships.rs`,
//! `iframe_loader_relationships.rs`, `html_files_inline_js.rs`, and
//! `discovered_lazy_loaded_js_files.rs` (REPORT_06 §7 "Relationship Graph
//! System"). No SQLite — this is the in-memory model usable for single-run
//! analysis or JSON export.
//!
//! ## What you get
//!
//! Given an HTML page, [`AssetGraph::from_html`] returns a graph with:
//!   - `<script src="...">` edges → external JS files (relationship = `JsLoader`)
//!   - inline `<script>...</script>` blocks (relationship = `InlineJs`)
//!   - `<iframe src="...">` edges → embedded HTML pages (relationship = `IframeLoader`)
//!   - `<link rel="modulepreload" href="...">` and `<link rel="preload" as="script">`
//!     (relationship = `LazyChunk`) — these are commonly used by Next.js / Vite
//!     to surface lazy-loaded chunks ahead of time.
//!   - source-map references (`//# sourceMappingURL=...`) inside inline scripts
//!     (relationship = `SourceMap`).
//!
//! The output schema is JSON-serializable so callers can dump it via
//! `--json` and pipe to ffuf, jq, or feed it back into the analyzer pipeline.
//!
//! ## What's *not* here
//!
//! No HTTP fetching — this is offline analysis only. To resolve relative
//! URLs against a base, callers should pass the page URL as `page_url`.

use lazy_static::lazy_static;
use regex::Regex;
use serde::{Deserialize, Serialize};

lazy_static! {
    static ref SCRIPT_TAG_RE: Regex =
        Regex::new(r#"(?si)<script(?P<attrs>(?:\s[^>]*)?)>(?P<body>.*?)</script>"#).unwrap();
    static ref IFRAME_TAG_RE: Regex = Regex::new(r#"(?si)<iframe(?P<attrs>\s[^>]*)?>"#).unwrap();
    static ref LINK_TAG_RE: Regex = Regex::new(r#"(?si)<link(?P<attrs>\s[^>]*)/?>"#).unwrap();
    static ref ATTR_SRC_RE: Regex = Regex::new(r#"(?i)\bsrc\s*=\s*["']([^"']+)["']"#).unwrap();
    static ref ATTR_HREF_RE: Regex = Regex::new(r#"(?i)\bhref\s*=\s*["']([^"']+)["']"#).unwrap();
    static ref ATTR_TYPE_RE: Regex = Regex::new(r#"(?i)\btype\s*=\s*["']([^"']+)["']"#).unwrap();
    static ref ATTR_REL_RE: Regex = Regex::new(r#"(?i)\brel\s*=\s*["']([^"']+)["']"#).unwrap();
    static ref ATTR_AS_RE: Regex = Regex::new(r#"(?i)\bas\s*=\s*["']([^"']+)["']"#).unwrap();
    static ref SOURCE_MAP_URL_RE: Regex = Regex::new(r#"(?im)//[#@]\s*sourceMappingURL\s*=\s*(\S+)"#).unwrap();
}

/// Kind of edge in the asset graph. Matches jxscout's relationship-table names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    /// `<script src="...">` external load.
    JsLoader,
    /// Inline `<script>...</script>` block.
    InlineJs,
    /// `<iframe src="...">` embed.
    IframeLoader,
    /// `<link rel="preload"|"modulepreload">` for JS chunks.
    LazyChunk,
    /// `//# sourceMappingURL=...` reference inside JS.
    SourceMap,
}

/// One edge in the graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetRelation {
    pub kind: RelationKind,
    /// Absolute URL when [`AssetGraph::from_html`] was given a `page_url`,
    /// otherwise the raw attribute value. May be empty for `InlineJs`.
    pub target: String,
    /// 1-indexed line number in the source HTML where this edge was seen.
    pub line: u32,
    /// For `InlineJs`: the actual script body (UTF-8). Empty otherwise.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub inline_content: String,
}

/// The full extracted graph for one HTML page.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AssetGraph {
    /// Page identifier — URL if known, else file path.
    pub page: String,
    /// All discovered edges.
    pub relations: Vec<AssetRelation>,
}

impl AssetGraph {
    /// Build an [`AssetGraph`] from a raw HTML body.
    ///
    /// `page` is used as the identifier in the output and as the base URL for
    /// resolving relative `src=` / `href=` values. Pass the absolute URL if
    /// you want absolute targets in the output; pass a file path if you only
    /// have the body on disk.
    #[must_use]
    pub fn from_html(html: &str, page: &str) -> Self {
        let mut relations: Vec<AssetRelation> = Vec::new();
        let base = url::Url::parse(page).ok();

        // <script src=...> + inline <script>...</script>
        for cap in SCRIPT_TAG_RE.captures_iter(html) {
            let attrs = cap.name("attrs").map_or("", |m| m.as_str());
            let body = cap.name("body").map_or("", |m| m.as_str());
            let line = byte_offset_to_line(html, cap.get(0).unwrap().start());

            let type_attr = ATTR_TYPE_RE
                .captures(attrs)
                .and_then(|c| c.get(1))
                .map(|m| m.as_str().to_ascii_lowercase());
            // Skip JSON/template scripts, but allow type="module" and type missing.
            let is_js = match type_attr.as_deref() {
                None | Some("") | Some("text/javascript") | Some("application/javascript") | Some("module") => true,
                _ => false,
            };
            if !is_js {
                continue;
            }

            if let Some(src_cap) = ATTR_SRC_RE.captures(attrs) {
                let raw_src = src_cap.get(1).unwrap().as_str();
                let target = resolve(&base, raw_src);
                relations.push(AssetRelation {
                    kind: RelationKind::JsLoader,
                    target,
                    line,
                    inline_content: String::new(),
                });
            } else if !body.trim().is_empty() {
                relations.push(AssetRelation {
                    kind: RelationKind::InlineJs,
                    target: String::new(),
                    line,
                    inline_content: body.to_string(),
                });

                // Source-map references inside inline blocks.
                for sm in SOURCE_MAP_URL_RE.captures_iter(body) {
                    let url = sm.get(1).unwrap().as_str().trim_matches(|c| c == '"' || c == '\'');
                    relations.push(AssetRelation {
                        kind: RelationKind::SourceMap,
                        target: resolve(&base, url),
                        line,
                        inline_content: String::new(),
                    });
                }
            }
        }

        // <iframe src=...>
        for cap in IFRAME_TAG_RE.captures_iter(html) {
            let attrs = cap.name("attrs").map_or("", |m| m.as_str());
            let line = byte_offset_to_line(html, cap.get(0).unwrap().start());
            if let Some(src_cap) = ATTR_SRC_RE.captures(attrs) {
                let raw = src_cap.get(1).unwrap().as_str();
                relations.push(AssetRelation {
                    kind: RelationKind::IframeLoader,
                    target: resolve(&base, raw),
                    line,
                    inline_content: String::new(),
                });
            }
        }

        // <link rel="preload" as="script" href=...> and <link rel="modulepreload" href=...>
        for cap in LINK_TAG_RE.captures_iter(html) {
            let attrs = cap.name("attrs").map_or("", |m| m.as_str());
            let line = byte_offset_to_line(html, cap.get(0).unwrap().start());

            let rel = ATTR_REL_RE
                .captures(attrs)
                .and_then(|c| c.get(1))
                .map(|m| m.as_str().to_ascii_lowercase());
            let as_attr = ATTR_AS_RE
                .captures(attrs)
                .and_then(|c| c.get(1))
                .map(|m| m.as_str().to_ascii_lowercase());

            let is_lazy_chunk = match rel.as_deref() {
                Some("modulepreload") => true,
                Some("preload") if as_attr.as_deref() == Some("script") => true,
                _ => false,
            };
            if !is_lazy_chunk {
                continue;
            }
            if let Some(href_cap) = ATTR_HREF_RE.captures(attrs) {
                let raw = href_cap.get(1).unwrap().as_str();
                relations.push(AssetRelation {
                    kind: RelationKind::LazyChunk,
                    target: resolve(&base, raw),
                    line,
                    inline_content: String::new(),
                });
            }
        }

        Self {
            page: page.to_string(),
            relations,
        }
    }

    /// Filter relations by kind.
    pub fn of_kind(&self, kind: RelationKind) -> impl Iterator<Item = &AssetRelation> {
        self.relations.iter().filter(move |r| r.kind == kind)
    }

    /// All `<script src>` targets — what jxscout's `get-loaded-js-files` returns.
    #[must_use]
    pub fn loaded_js_files(&self) -> Vec<&str> {
        self.of_kind(RelationKind::JsLoader)
            .map(|r| r.target.as_str())
            .collect()
    }

    /// All `<iframe>` targets — what `get-loaded-iframes` returns.
    #[must_use]
    pub fn loaded_iframes(&self) -> Vec<&str> {
        self.of_kind(RelationKind::IframeLoader)
            .map(|r| r.target.as_str())
            .collect()
    }

    /// All lazy-loaded chunks discovered via `<link rel=preload|modulepreload>`.
    #[must_use]
    pub fn lazy_chunks(&self) -> Vec<&str> {
        self.of_kind(RelationKind::LazyChunk)
            .map(|r| r.target.as_str())
            .collect()
    }
}

fn resolve(base: &Option<url::Url>, raw: &str) -> String {
    match base {
        Some(b) => match b.join(raw) {
            Ok(joined) => joined.to_string(),
            Err(_) => raw.to_string(),
        },
        None => raw.to_string(),
    }
}

fn byte_offset_to_line(s: &str, offset: usize) -> u32 {
    let limit = offset.min(s.len());
    let mut line = 1u32;
    for (i, b) in s.as_bytes().iter().enumerate() {
        if i >= limit {
            break;
        }
        if *b == b'\n' {
            line += 1;
        }
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE_HTML: &str = r#"<!doctype html>
<html>
<head>
    <link rel="modulepreload" href="/_next/static/chunks/main.js">
    <link rel="preload" as="script" href="/static/runtime.js">
    <link rel="stylesheet" href="/style.css">
    <script src="https://cdn.example.com/lib.js"></script>
    <script src="/static/app.js"></script>
</head>
<body>
    <iframe src="https://embed.example.com/widget"></iframe>
    <iframe src="/internal/frame"></iframe>
    <script>
        const apiKey = "ghp_xxx";
        //# sourceMappingURL=app.js.map
    </script>
    <script type="application/json">{"not":"js"}</script>
</body>
</html>"#;

    #[test]
    fn extracts_loaded_js_files() {
        let g = AssetGraph::from_html(PAGE_HTML, "https://target.com/app");
        let js = g.loaded_js_files();
        assert!(js.contains(&"https://cdn.example.com/lib.js"));
        assert!(js.contains(&"https://target.com/static/app.js")); // resolved
    }

    #[test]
    fn extracts_iframes() {
        let g = AssetGraph::from_html(PAGE_HTML, "https://target.com/app");
        let frames = g.loaded_iframes();
        assert!(frames.contains(&"https://embed.example.com/widget"));
        assert!(frames.contains(&"https://target.com/internal/frame"));
    }

    #[test]
    fn extracts_lazy_chunks() {
        let g = AssetGraph::from_html(PAGE_HTML, "https://target.com/app");
        let chunks = g.lazy_chunks();
        assert_eq!(chunks.len(), 2);
        assert!(chunks.iter().any(|c| c.ends_with("/main.js")));
        assert!(chunks.iter().any(|c| c.ends_with("/runtime.js")));
    }

    #[test]
    fn skips_non_js_script_blocks() {
        let g = AssetGraph::from_html(PAGE_HTML, "https://target.com/");
        // The <script type="application/json"> should NOT be in inline_js
        let inline: Vec<_> = g.of_kind(RelationKind::InlineJs).collect();
        assert_eq!(inline.len(), 1);
        assert!(inline[0].inline_content.contains("apiKey"));
    }

    #[test]
    fn extracts_sourcemap_reference_in_inline_js() {
        let g = AssetGraph::from_html(PAGE_HTML, "https://target.com/app");
        let maps: Vec<_> = g.of_kind(RelationKind::SourceMap).collect();
        assert_eq!(maps.len(), 1);
        assert!(maps[0].target.ends_with("app.js.map"));
    }

    #[test]
    fn relative_resolution_without_base() {
        // No page URL → leaves relative paths as-is.
        let g = AssetGraph::from_html(r#"<script src="/x.js"></script>"#, "/local/file.html");
        assert_eq!(g.loaded_js_files(), vec!["/x.js"]);
    }

    #[test]
    fn json_round_trip() {
        let g = AssetGraph::from_html(PAGE_HTML, "https://target.com/app");
        let s = serde_json::to_string(&g).unwrap();
        assert!(s.contains(r#""kind":"js_loader""#));
        let back: AssetGraph = serde_json::from_str(&s).unwrap();
        assert_eq!(back.relations.len(), g.relations.len());
    }
}
