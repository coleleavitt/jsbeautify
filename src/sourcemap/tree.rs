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
    let map: EmbeddedSources = serde_json::from_str(map_json)?;
    let mut out = Vec::with_capacity(map.sources.len());
    map.collect(&mut out);
    Ok(out)
}

/// The parts of a source map [`reverse`] reads. An index map (Turbopack
/// writes only these) has no `mappings` or `names` of its own; its
/// `sections` each hold a regular map.
#[derive(serde::Deserialize)]
struct EmbeddedSources {
    #[serde(default)]
    sources: Vec<String>,
    #[serde(default, rename = "sourcesContent")]
    sources_content: Option<Vec<Option<String>>>,
    #[serde(default)]
    sections: Vec<Section>,
}

#[derive(serde::Deserialize)]
struct Section {
    map: EmbeddedSources,
}

impl EmbeddedSources {
    fn collect(self, out: &mut Vec<ReversedSource>) {
        for (src, content) in self.sources.into_iter().zip(self.sources_content.unwrap_or_default()) {
            if let Some(content) = content.filter(|c| !c.is_empty()) {
                out.push(ReversedSource {
                    source_path: src,
                    content,
                });
            }
        }
        for section in self.sections {
            section.map.collect(out);
        }
    }
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
    if let Some(idx) = s.find("://") {
        let after = &s[idx + 3..];
        s = after.split_once('/').map_or("", |(_, p)| p);
    }
    let s = s.split(['?', '#']).next().unwrap_or(s);

    let mut segments: Vec<&str> = Vec::new();
    for seg in s.split(['/', '\\']) {
        match seg {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
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
    write_reversed_sources(sources, output_dir)
}

pub(super) fn write_reversed_sources(
    sources: impl IntoIterator<Item = ReversedSource>,
    output_dir: &std::path::Path,
) -> Result<Vec<String>, String> {
    let root = output_dir.join("reversed");
    let mut written = Vec::new();
    for src in sources {
        let rel = normalize_source_path(&src.source_path);
        let target = root.join(&rel);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverse_reads_index_map_sections() {
        let map = r#"{"version":3,"sources":[],"sections":[
            {"offset":{"line":0,"column":0},"map":{"version":3,"sources":[],"names":[],"mappings":""}},
            {"offset":{"line":3,"column":0},"map":{"version":3,"sources":["turbopack:///[project]/src/a.ts"],"sourcesContent":["export const a = 1;"],"names":[],"mappings":"AAAA"}}]}"#;
        let sources = reverse(map).expect("index map parses");
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].source_path, "turbopack:///[project]/src/a.ts");
        assert_eq!(sources[0].content, "export const a = 1;");
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
        assert_eq!(normalize_source_path("C:\\Users\\x\\app.ts"), "Users/x/app.ts");
    }

    #[test]
    fn path_traversal_cannot_escape_root() {
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
