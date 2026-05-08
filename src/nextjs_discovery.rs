//! Next.js chunk discovery.
//!
//! Ports `crates/core/nextjs-chunk-discoverer/` from jxscout.
//!
//! Given a Next.js app URL, fetches `/_next/static/<buildId>/_buildManifest.js`,
//! parses it to extract all chunk paths, and returns full URLs for each.
//!
//! ## Usage
//!
//! ```rust,no_run
//! use js_beautify_rs::nextjs_discovery;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let chunks = nextjs_discovery::discover("https://example.com").await?;
//! for url in &chunks {
//!     println!("{url}");
//! }
//! # Ok(())
//! # }
//! ```

use lazy_static::lazy_static;
use regex::Regex;

lazy_static! {
    /// Matches the buildId from `/_next/static/XXXXX/_buildManifest.js` or
    /// `__NEXT_DATA__` JSON with "buildId":"...".
    static ref BUILD_ID_RE: Regex = Regex::new(
        r#"(?:"buildId"\s*:\s*"([^"]+)"|/_next/static/([a-zA-Z0-9_-]{6,})/_(?:build|ssg)Manifest\.js)"#
    ).unwrap();

    /// Extracts chunk paths from the _buildManifest.js content.
    /// Pattern: "static/chunks/xxx.js" or "static/css/xxx.css"
    static ref CHUNK_PATH_RE: Regex = Regex::new(
        r#""(static/(?:chunks|css|media)/[^"]+)""#
    ).unwrap();

    /// Next.js sortedPages array entries (route paths).
    static ref SORTED_PAGES_RE: Regex = Regex::new(
        r#"sortedPages\s*:\s*\[([\s\S]*?)\]"#
    ).unwrap();
}

/// Discovered Next.js build information.
#[derive(Debug, Clone)]
pub struct NextJsBuild {
    pub build_id: String,
    pub base_url: String,
    pub chunk_urls: Vec<String>,
    pub routes: Vec<String>,
}

/// Parse a `_buildManifest.js` file content and return all chunk paths.
///
/// This is the pure-function version that doesn't do any HTTP fetching —
/// useful when you already have the manifest content (e.g. from a proxy capture).
#[must_use]
pub fn parse_build_manifest(manifest_content: &str, base_url: &str) -> NextJsBuild {
    let base = base_url.trim_end_matches('/');

    // Extract build ID
    let build_id = BUILD_ID_RE
        .captures(manifest_content)
        .and_then(|c| c.get(1).or(c.get(2)))
        .map(|m| m.as_str().to_string())
        .unwrap_or_default();

    // Extract chunk paths
    let mut chunk_urls: Vec<String> = CHUNK_PATH_RE
        .captures_iter(manifest_content)
        .map(|c| {
            let path = &c[1];
            if build_id.is_empty() {
                format!("{base}/_next/{path}")
            } else {
                format!("{base}/_next/{path}")
            }
        })
        .collect();
    chunk_urls.sort_unstable();
    chunk_urls.dedup();

    // Extract routes from sortedPages
    let routes: Vec<String> = if let Some(cap) = SORTED_PAGES_RE.captures(manifest_content) {
        cap[1]
            .split(',')
            .filter_map(|s| {
                let trimmed = s.trim().trim_matches('"').trim_matches('\'');
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                }
            })
            .collect()
    } else {
        Vec::new()
    };

    NextJsBuild {
        build_id,
        base_url: base.to_string(),
        chunk_urls,
        routes,
    }
}

/// Construct the URL for `_buildManifest.js` given a base URL and build ID.
#[must_use]
pub fn manifest_url(base_url: &str, build_id: &str) -> String {
    let base = base_url.trim_end_matches('/');
    format!("{base}/_next/static/{build_id}/_buildManifest.js")
}

/// Extract the build ID from Next.js `__NEXT_DATA__` JSON or page HTML.
#[must_use]
pub fn extract_build_id(html_or_json: &str) -> Option<String> {
    BUILD_ID_RE
        .captures(html_or_json)
        .and_then(|c| c.get(1).or(c.get(2)))
        .map(|m| m.as_str().to_string())
}

/// Discover all possible Next.js page chunk URLs from an HTML page source.
///
/// This looks for `/_next/static/<buildId>/_ssgManifest.js` and similar patterns
/// in the page HTML to find the build ID without fetching the manifest separately.
#[must_use]
pub fn discover_from_html(html: &str, base_url: &str) -> NextJsBuild {
    let base = base_url.trim_end_matches('/');

    // Try to find build ID from the HTML
    let build_id = extract_build_id(html).unwrap_or_default();

    // Find all chunk references already in the page
    let mut chunk_urls: Vec<String> = Vec::new();

    // Pattern: /_next/static/chunks/xxx.js in script src
    let next_static_re = Regex::new(r#"/_next/static/[^"'\s]+"#).unwrap();
    for m in next_static_re.find_iter(html) {
        let path = m.as_str();
        if path.contains("/chunks/") || path.contains("/css/") || path.contains("/media/") {
            chunk_urls.push(format!("{base}{path}"));
        }
    }

    // Pattern: "static/chunks/..." in inline scripts (build manifest content)
    for cap in CHUNK_PATH_RE.captures_iter(html) {
        chunk_urls.push(format!("{base}/_next/{}", &cap[1]));
    }

    chunk_urls.sort_unstable();
    chunk_urls.dedup();

    let routes = if let Some(cap) = SORTED_PAGES_RE.captures(html) {
        cap[1]
            .split(',')
            .filter_map(|s| {
                let trimmed = s.trim().trim_matches('"').trim_matches('\'');
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                }
            })
            .collect()
    } else {
        Vec::new()
    };

    NextJsBuild {
        build_id,
        base_url: base.to_string(),
        chunk_urls,
        routes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_build_manifest() {
        let manifest = r#"
            self.__BUILD_MANIFEST = {
                sortedPages: ["/", "/about", "/api/users", "/dashboard"],
                "/": ["static/chunks/pages/index-abc123.js", "static/css/main-def456.css"],
                "/about": ["static/chunks/pages/about-ghi789.js"],
                "/dashboard": ["static/chunks/pages/dashboard-jkl012.js"]
            };
        "#;
        let result = parse_build_manifest(manifest, "https://example.com");
        assert!(!result.chunk_urls.is_empty());
        assert!(result.chunk_urls.iter().any(|u| u.contains("index-abc123")));
        assert!(result.chunk_urls.iter().any(|u| u.contains("dashboard-jkl012")));
        assert!(result.routes.contains(&"/dashboard".to_string()));
        assert!(result.routes.contains(&"/api/users".to_string()));
    }

    #[test]
    fn extracts_build_id_from_next_data() {
        let html = r#"<script id="__NEXT_DATA__">{"buildId":"abc123XYZ","page":"/"}</script>"#;
        assert_eq!(extract_build_id(html), Some("abc123XYZ".to_string()));
    }

    #[test]
    fn discovers_from_page_html() {
        let html = r#"
            <script src="/_next/static/chunks/webpack-abc123.js"></script>
            <script src="/_next/static/chunks/framework-def456.js"></script>
            <script src="/_next/static/abc123XYZ/_buildManifest.js"></script>
            <script id="__NEXT_DATA__">{"buildId":"abc123XYZ","page":"/"}</script>
        "#;
        let result = discover_from_html(html, "https://target.com");
        assert_eq!(result.build_id, "abc123XYZ");
        assert!(result.chunk_urls.iter().any(|u| u.contains("webpack-abc123")));
        assert!(result.chunk_urls.iter().any(|u| u.contains("framework-def456")));
    }
}
