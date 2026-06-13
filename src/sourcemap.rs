use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceMap {
    pub version: u32,
    pub sources: Vec<String>,
    pub names: Vec<String>,
    pub mappings: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_root: Option<String>,
    /// Embedded original sources (`sourcesContent` in spec). When present,
    /// each entry corresponds 1:1 with `sources[i]`. Used by [`reverse`].
    #[serde(default, rename = "sourcesContent", skip_serializing_if = "Option::is_none")]
    pub sources_content: Option<Vec<Option<String>>>,
}

/// One reversed source file extracted from a sourcemap's `sourcesContent`.
///
/// jxscout writes each entry to disk with a path like
/// `<output_dir>/<source_path>` and re-runs the analyzer pipeline against it
/// using `FileType::ReversedSource` so findings can be attributed back to the
/// original file (TS/JSX/etc.) rather than the bundled JS.
#[derive(Debug, Clone)]
pub struct ReversedSource {
    /// Path string from the map's `sources` array.
    pub source_path: String,
    /// Original source code, taken from `sourcesContent[i]`.
    pub content: String,
}

/// Parse a `.map` JSON blob and extract every `(source_path, content)` pair
/// that has a non-null entry in `sourcesContent`.
///
/// Returns an empty vec if the map has no `sourcesContent` (older or
/// privacy-stripped maps). Caller can then fall back to fetching the source
/// over HTTP — which is what jxscout's `sourcemaps` crate does — but even the
/// embedded path is useful for surface mapping.
///
/// # Errors
/// Returns an error if `map_json` is not a valid JSON sourcemap.
pub fn reverse(map_json: &str) -> Result<Vec<ReversedSource>, serde_json::Error> {
    let map: SourceMap = serde_json::from_str(map_json)?;
    let contents = map.sources_content.unwrap_or_default();
    let mut out = Vec::with_capacity(map.sources.len());
    for (i, src) in map.sources.iter().enumerate() {
        if let Some(Some(content)) = contents.get(i)
            && !content.is_empty()
        {
            out.push(ReversedSource {
                source_path: src.clone(),
                content: content.clone(),
            });
        }
    }
    Ok(out)
}

/// Normalize a sourcemap `sources[]` path into a **safe relative path** suitable
/// for writing under an output directory.
///
/// Bundler source paths come in many shapes:
/// - `webpack://my-app/./src/index.ts`
/// - `webpack://_N_E/./components/Button.tsx`
/// - `rollup:///src/main.js`, `vite:///@fs/...`
/// - absolute URLs (`https://app/static/x.js`) or absolute FS paths (`/home/...`)
/// - `../../node_modules/...` traversal
///
/// We strip the scheme + bundler-namespace authority, drop `webpack-internal:`
/// and `?` query suffixes, collapse `./` and `../` so nothing escapes the output
/// root, and guarantee the result is relative (never starts with `/` or a drive).
#[must_use]
pub fn normalize_source_path(raw: &str) -> String {
    let mut s = raw.trim();

    // Strip `webpack-internal:///` and similar internal prefixes entirely.
    for prefix in [
        "webpack-internal://",
        "webpack://",
        "rollup://",
        "vite://",
        "turbopack://",
    ] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest;
            break;
        }
    }
    // Strip a generic `scheme://authority/` for http(s)/file URLs.
    if let Some(idx) = s.find("://") {
        let after = &s[idx + 3..];
        s = after.split_once('/').map_or("", |(_, p)| p);
    }
    // Drop query/hash suffixes (e.g. `?v=hash`).
    let s = s.split(['?', '#']).next().unwrap_or(s);

    // Some webpack namespaces look like `_N_E/./src/x` or `./src/x`.
    let mut segments: Vec<&str> = Vec::new();
    for seg in s.split(['/', '\\']) {
        match seg {
            "" | "." => {}
            ".." => {
                // Never escape the root: pop if possible, else ignore.
                segments.pop();
            }
            // Drop Windows drive letters like `C:`.
            d if d.len() == 2 && d.ends_with(':') && d.as_bytes()[0].is_ascii_alphabetic() => {}
            other => segments.push(other),
        }
    }
    let joined = segments.join("/");
    if joined.is_empty() {
        "unknown_source".to_string()
    } else {
        joined
    }
}

/// Reconstruct a source-map's original file tree under `output_dir/reversed/`.
///
/// Parses `map_json`, normalizes every `sources[]` path to a safe relative path,
/// and writes each `sourcesContent[i]` to `output_dir/reversed/<rel_path>`,
/// creating intermediate directories. Returns the relative paths written.
///
/// # Errors
/// I/O errors creating directories / writing files, or a malformed map.
pub fn reconstruct_tree(map_json: &str, output_dir: &std::path::Path) -> Result<Vec<String>, String> {
    let sources = reverse(map_json).map_err(|e| format!("parse sourcemap: {e}"))?;
    let root = output_dir.join("reversed");
    let mut written = Vec::new();
    for src in sources {
        let rel = normalize_source_path(&src.source_path);
        let target = root.join(&rel);
        // Defense in depth: ensure the resolved path stays under `root`.
        if !target.starts_with(&root) {
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }
        std::fs::write(&target, src.content.as_bytes()).map_err(|e| format!("write {}: {e}", target.display()))?;
        written.push(rel);
    }
    Ok(written)
}

impl SourceMap {
    #[must_use]
    pub fn new(source_file: &str) -> Self {
        Self {
            version: 3,
            sources: vec![source_file.to_string()],
            names: vec![],
            mappings: String::new(),
            file: None,
            source_root: None,
            sources_content: None,
        }
    }

    pub fn add_simple_mapping(&mut self, generated_line: usize, original_line: usize) {
        if generated_line == 0 {
            return;
        }

        let line_offset = i64::try_from(original_line).unwrap_or(i64::MAX) - 1;
        let encoded = encode_vlq(0) + &encode_vlq(0) + &encode_vlq(line_offset) + &encode_vlq(0);

        if !self.mappings.is_empty() {
            self.mappings.push(';');
        }
        self.mappings.push_str(&encoded);
    }

    /// # Errors
    /// Returns an error if the operation fails.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// # Errors
    /// Returns an error if the operation fails.
    pub fn to_data_url(&self) -> Result<String, serde_json::Error> {
        let json = self.to_json()?;
        let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, json);
        Ok(format!("data:application/json;base64,{encoded}"))
    }

    #[must_use]
    pub fn for_chunk(chunk_name: &str, chunk_filename: &str, line_count: usize) -> Self {
        let mut map = Self::new(chunk_name);
        map.file = Some(chunk_filename.to_string());

        for line in 1..=line_count {
            map.add_simple_mapping(line, line);
        }

        map
    }
}

const VLQ_BASE: i64 = 32;
const VLQ_BASE_MASK: i64 = VLQ_BASE - 1;
const VLQ_CONTINUATION_BIT: i64 = VLQ_BASE;

fn encode_vlq(value: i64) -> String {
    let mut vlq = if value < 0 { ((-value) << 1) | 1 } else { value << 1 };

    let mut result = String::new();
    loop {
        let mut digit = vlq & VLQ_BASE_MASK;
        vlq >>= 5;
        if vlq > 0 {
            digit |= VLQ_CONTINUATION_BIT;
        }
        result.push(encode_base64_digit(digit));
        if vlq == 0 {
            break;
        }
    }
    result
}

fn encode_base64_digit(digit: i64) -> char {
    let chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    chars.chars().nth(digit as usize).unwrap_or('A')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_sourcemap() {
        let map = SourceMap::new("input.js");
        assert_eq!(map.version, 3);
        assert_eq!(map.sources.len(), 1);
        assert_eq!(map.sources[0], "input.js");
    }

    #[test]
    fn normalizes_bundler_source_paths() {
        assert_eq!(
            normalize_source_path("webpack://my-app/./src/index.ts"),
            "my-app/src/index.ts"
        );
        assert_eq!(
            normalize_source_path("webpack://_N_E/./components/Button.tsx"),
            "_N_E/components/Button.tsx"
        );
        assert_eq!(normalize_source_path("rollup:///src/main.js"), "src/main.js");
        assert_eq!(normalize_source_path("webpack-internal:///./util.js"), "util.js");
        assert_eq!(
            normalize_source_path("https://app.com/static/app.js?v=abc"),
            "static/app.js"
        );
        // Drive letter + backslashes.
        assert_eq!(normalize_source_path("C:\\Users\\x\\app.ts"), "Users/x/app.ts");
    }

    #[test]
    fn path_traversal_cannot_escape_root() {
        // `..` segments are popped, never allowed to climb out.
        assert_eq!(normalize_source_path("../../etc/passwd"), "etc/passwd");
        assert_eq!(normalize_source_path("/abs/../../x"), "x");
        assert_eq!(normalize_source_path("webpack://app/../../../secret"), "secret");
        assert_eq!(normalize_source_path("./"), "unknown_source");
    }

    #[test]
    fn reconstruct_tree_writes_files() {
        let dir = std::env::temp_dir().join(format!("jsb-smtest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let map = serde_json::json!({
            "version": 3,
            "sources": ["webpack://app/./src/a.ts", "webpack://app/./src/util/b.ts"],
            "sourcesContent": ["export const a = 1;", "export const b = 2;"],
            "names": [],
            "mappings": ""
        })
        .to_string();

        let written = reconstruct_tree(&map, &dir).unwrap();
        assert_eq!(written.len(), 2);
        let a = dir.join("reversed/app/src/a.ts");
        let b = dir.join("reversed/app/src/util/b.ts");
        assert!(a.exists() && b.exists());
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "export const a = 1;");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_add_simple_mapping() {
        let mut map = SourceMap::new("input.js");
        map.add_simple_mapping(1, 1);
        assert!(!map.mappings.is_empty());
    }

    #[test]
    fn test_to_json() {
        let map = SourceMap::new("input.js");
        let json = map.to_json().unwrap();
        assert!(json.contains("\"version\":3"));
        assert!(json.contains("input.js"));
    }

    #[test]
    fn test_for_chunk() {
        let map = SourceMap::for_chunk("LoginView", "LoginView.chunk.363842.js", 10);

        assert_eq!(map.version, 3);
        assert_eq!(map.sources.len(), 1);
        assert_eq!(map.sources[0], "LoginView");
        assert_eq!(map.file, Some("LoginView.chunk.363842.js".to_string()));
        assert!(!map.mappings.is_empty());
    }

    #[test]
    fn test_reverse_extracts_sources() {
        let map_json = r#"{
            "version": 3,
            "sources": ["src/login.ts", "src/api.ts"],
            "sourcesContent": [
                "export function login() { return fetch('/api/auth'); }",
                "export const API = '/api/v2';"
            ],
            "names": [],
            "mappings": ""
        }"#;
        let reversed = reverse(map_json).unwrap();
        assert_eq!(reversed.len(), 2);
        assert_eq!(reversed[0].source_path, "src/login.ts");
        assert!(reversed[0].content.contains("login"));
        assert_eq!(reversed[1].source_path, "src/api.ts");
    }

    #[test]
    fn test_reverse_skips_null_content() {
        let map_json = r#"{
            "version": 3,
            "sources": ["a.ts", "b.ts"],
            "sourcesContent": [null, "export const x = 1;"],
            "names": [],
            "mappings": ""
        }"#;
        let reversed = reverse(map_json).unwrap();
        assert_eq!(reversed.len(), 1);
        assert_eq!(reversed[0].source_path, "b.ts");
    }

    #[test]
    fn test_reverse_empty_when_no_sources_content() {
        let map_json = r#"{
            "version": 3,
            "sources": ["a.ts"],
            "names": [],
            "mappings": ""
        }"#;
        let reversed = reverse(map_json).unwrap();
        assert!(reversed.is_empty());
    }
}
