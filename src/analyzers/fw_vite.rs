//! Vite recon analyzer. Detects Vite build/runtime fingerprints that survive
//! minification in production bundles (preload helpers, dep maps, env vars,
//! and leaked dev-server markers).
//!
//! Signatures that survive minification in production bundles; emits AnalyzerMatch.
//!
//! Signatures confirmed against the cloned Vite source:
//!   - `__vitePreload` / `vite:preloadError` / `__vite__mapDeps`:
//!     packages/vite/src/node/plugins/importAnalysisBuild.ts
//!       (preloadMethod L30, `vite:preloadError` Event L146,
//!        `__vite__mapDeps([...])` L475, mapDeps codegen L490)
//!   - `import.meta.env` / `VITE_` prefix: packages/vite/src/node/env.ts (L36, L96, L110)
//!   - dev markers `/@vite/client`, `/@vite/env`, `/@fs/`, `vite-hmr`:
//!     packages/vite/src/node/constants.ts (FS_PREFIX L119, CLIENT_PUBLIC_PATH L121,
//!     ENV_PUBLIC_PATH L122), packages/vite/src/node/server/ws.ts (HMR_HEADER L34),
//!     packages/vite/src/client/client.ts (L58, L74)

use super::match_kind::{AnalyzerMatch, FileType, PositionRange, Provenance};
use super::regex_bank::Severity;
use lazy_static::lazy_static;
use regex::Regex;

lazy_static! {
    /// `new Event('vite:preloadError', ...)` — emitted by the runtime preload
    /// helper. The literal string survives minification verbatim.
    /// importAnalysisBuild.ts L146.
    static ref PRELOAD_ERROR_RE: Regex = Regex::new(r#"vite:preloadError"#).unwrap();

    /// `const __vite__mapDeps=(i,m=__vite__mapDeps,d=(m.f||(m.f=[...])))=>i.map(...)`
    /// Older bundles emit `(i,m,d=(...))`. Capture group 1 is the chunk+CSS table.
    /// importAnalysisBuild.ts L490 (codegen) / L484 (fileDepsCode).
    static ref DEP_MAP_RE: Regex = Regex::new(
        r#"__vite__mapDeps=\(i,m(?:=__vite__mapDeps)?,d=\(m\.f\|\|\(m\.f=(\[[^\]]*\])\)\)\)"#,
    )
    .unwrap();

    /// `__vitePreload(()=>import(X), __vite__mapDeps([i,j]))` lazy-import graph.
    /// Capture group 1 is the dynamic import target. importAnalysisBuild.ts L179.
    static ref PRELOAD_RE: Regex =
        Regex::new(r#"__vitePreload\(\s*\(\)\s*=>\s*import\(([^)]+)\)"#).unwrap();

    /// `import.meta.env.VITE_FOO` or `import.meta.env["VITE_FOO"]`. Only the
    /// public `VITE_`-prefixed keys are client-exposed. env.ts L36/L96/L110.
    static ref ENV_VAR_RE: Regex = Regex::new(
        r#"import\.meta\.env(?:\.(VITE_[A-Za-z0-9_]+)|\[\s*["'](VITE_[A-Za-z0-9_]+)["']\s*\])"#,
    )
    .unwrap();

    /// Dev-server internal paths / HMR header. Their presence in a production
    /// bundle indicates a leaked dev build. constants.ts L119/L121/L122, ws.ts L34.
    static ref DEV_MARKER_RE: Regex =
        Regex::new(r#"(/@vite/client|/@vite/env|/@fs/|vite-hmr)"#).unwrap();
}

/// Extract the entries of a flat JS string-array literal (`["a.js","b.css"]`).
///
/// Parses each quoted string literal directly rather than splitting on commas,
/// so a comma *inside* a quoted entry (a valid, if rare, path) does not corrupt
/// the table. Both single- and double-quote styles are handled.
fn parse_dep_array(arr: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = arr.chars();
    while let Some(c) = chars.next() {
        if c == '"' || c == '\'' {
            let quote = c;
            let mut entry = String::new();
            for ch in chars.by_ref() {
                if ch == quote {
                    break;
                }
                entry.push(ch);
            }
            if !entry.is_empty() {
                out.push(entry);
            }
        }
    }
    out
}

/// Scan source for Vite signatures.
#[must_use]
pub fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    let mut out = Vec::new();
    let fp = || file_path.map(str::to_string);

    // vite_fingerprint — runtime preload error event name.
    for m in PRELOAD_ERROR_RE.find_iter(source) {
        out.push(
            AnalyzerMatch::new(
                "vite_fingerprint",
                m.as_str(),
                PositionRange::from_byte_range(source, m.start(), m.end()),
                file_type,
                fp(),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info),
        );
    }

    // vite_dep_map — chunk + CSS dependency table.
    for caps in DEP_MAP_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let arr = caps.get(1).map_or("", |m| m.as_str());
        let chunks = parse_dep_array(arr);
        out.push(
            AnalyzerMatch::new(
                "vite_dep_map",
                arr.to_string(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                fp(),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info)
            .with_metadata(serde_json::json!({ "chunks": chunks })),
        );
    }

    // vite_preload — lazy dynamic-import graph.
    for caps in PRELOAD_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let target = caps
            .get(1)
            .map_or("", |m| m.as_str())
            .trim()
            .trim_matches(|c| c == '"' || c == '\'')
            .to_string();
        out.push(
            AnalyzerMatch::new(
                "vite_preload",
                target.clone(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                fp(),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info)
            .with_metadata(serde_json::json!({ "import": target })),
        );
    }

    // vite_env_var — public VITE_-prefixed env keys baked into the bundle.
    for caps in ENV_VAR_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let key = caps
            .get(1)
            .or_else(|| caps.get(2))
            .map_or("", |m| m.as_str())
            .to_string();
        out.push(
            AnalyzerMatch::new(
                "vite_env_var",
                key.clone(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                fp(),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Low)
            .with_metadata(serde_json::json!({ "env_key": key })),
        );
    }

    // vite_dev_marker — leaked dev-server internal paths / HMR header.
    for caps in DEV_MARKER_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let marker = caps.get(1).map_or("", |m| m.as_str()).to_string();
        out.push(
            AnalyzerMatch::new(
                "vite_dev_marker",
                marker,
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                fp(),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Low),
        );
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_dep_map_and_preload_and_fingerprint() {
        // Minified shapes derived from importAnalysisBuild.ts L490 / L475 / L146.
        let src = concat!(
            r#"const __vite__mapDeps=(i,m=__vite__mapDeps,d=(m.f||(m.f=["#,
            r#""assets/Home-a1b2c3.js","assets/Home-d4e5f6.css"])))=>i.map(i=>d[i]);"#,
            "\n",
            r#"e.exports=__vitePreload(()=>import("./assets/Home-a1b2c3.js"),__vite__mapDeps([0,1]));"#,
            "\n",
            r#"const t=new Event("vite:preloadError",{cancelable:!0});"#,
        );
        let m = analyze(src, Some("t.js"), FileType::Js);

        let dep = m.iter().find(|x| x.match_kind == "vite_dep_map").unwrap();
        assert!(dep.match_value.contains("assets/Home-a1b2c3.js"));
        assert!(dep.match_value.contains("assets/Home-d4e5f6.css"));

        let pre = m.iter().find(|x| x.match_kind == "vite_preload").unwrap();
        assert_eq!(pre.match_value, "./assets/Home-a1b2c3.js");

        assert!(m.iter().any(|x| x.match_kind == "vite_fingerprint"));
    }

    #[test]
    fn detects_env_var_dot_and_bracket() {
        let src = r#"const a=import.meta.env.VITE_API_URL,b=import.meta.env["VITE_PUBLIC_KEY"];"#;
        let m = analyze(src, Some("t.js"), FileType::Js);
        let keys: Vec<_> = m
            .iter()
            .filter(|x| x.match_kind == "vite_env_var")
            .map(|x| x.match_value.as_str())
            .collect();
        assert!(keys.contains(&"VITE_API_URL"));
        assert!(keys.contains(&"VITE_PUBLIC_KEY"));
        assert!(
            m.iter()
                .any(|x| x.match_kind == "vite_env_var" && x.severity == Severity::Low)
        );
    }

    #[test]
    fn detects_dev_markers() {
        let src = r#"fetch("/@vite/client");import"/@vite/env";const h="/@fs/home/app",x="vite-hmr";"#;
        let m = analyze(src, None, FileType::Js);
        let markers: Vec<_> = m
            .iter()
            .filter(|x| x.match_kind == "vite_dev_marker")
            .map(|x| x.match_value.as_str())
            .collect();
        assert!(markers.contains(&"/@vite/client"));
        assert!(markers.contains(&"/@fs/"));
        assert!(markers.contains(&"vite-hmr"));
    }
}
