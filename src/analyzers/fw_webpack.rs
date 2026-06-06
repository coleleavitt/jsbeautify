//! Webpack 5 runtime recon analyzer.
//!
//! Surfaces the webpack signatures that survive minification in production
//! bundles and reveal app structure:
//!   - the JSONP chunk-loading global `(self|window).webpackChunk<uniqueName>`
//!     (the `<uniqueName>` is the build's library/output name — e.g. Next.js
//!     emits `webpackChunk_N_E`);
//!   - the `__webpack_require__` runtime + `__webpack_modules__` registry;
//!   - the `.u`/`.miniCssF` chunk-filename functions' embedded
//!     `{chunkId:"chunkName"}` map — the single highest-value webpack artifact,
//!     because route/vendor/component chunk names survive verbatim even when
//!     everything else is mangled.
//!
//! These are property names / string literals webpack writes literally
//! (`RuntimeGlobals` in webpack/lib/RuntimeGlobals.js), so they are stable
//! across minification.

use super::match_kind::{AnalyzerMatch, FileType, PositionRange, Provenance};
use super::regex_bank::Severity;
use lazy_static::lazy_static;
use regex::Regex;

lazy_static! {
    /// `(self|window|globalThis|this).webpackChunk<uniqueName> =`. Group 1 = the
    /// build's unique chunk-loading global name (library/output.uniqueName).
    static ref JSONP_RE: Regex =
        Regex::new(r#"(?:self|window|globalThis|this)\.(webpackChunk[A-Za-z0-9_$]*)"#).unwrap();

    /// The webpack runtime symbols (literal even in mangled bundles).
    static ref RUNTIME_RE: Regex =
        Regex::new(r#"(__webpack_require__|__webpack_modules__|__webpack_exports__)"#).unwrap();

    /// A chunk-filename function `X.u = ...` / `X.miniCssF = ...` / `X.k = ...`.
    /// Captures the function tail up to the statement end so the embedded
    /// `{id:"name"}` map(s) can be mined from it. `.u` => JS chunk URL,
    /// `.miniCssF` => CSS chunk URL, `.k` => hot-update filename.
    static ref CHUNK_FN_RE: Regex =
        Regex::new(r#"\.(u|miniCssF|k)\s*=\s*(?:function\s*\([^)]*\)|[A-Za-z0-9_$]+\s*=>|\([^)]*\)\s*=>)([^;\n]{0,2000})"#)
            .unwrap();

    /// A `chunkId:"chunkName"` pair inside a chunk map. Keys may be bare numbers
    /// or quoted; values are quoted chunk names. Group 1 = id, group 2 = name.
    static ref CHUNK_PAIR_RE: Regex =
        Regex::new(r#"(?:(\d+)|"([^"]+)")\s*:\s*"([^"]+)""#).unwrap();
}

/// A chunk-name value is a real name (vs. a content hash) if it carries a
/// path-ish / wordy character. Pure `[0-9a-f]+` hashes are skipped.
fn looks_like_chunk_name(v: &str) -> bool {
    v.contains(['-', '/', '_', '.']) || v.chars().any(|c| c.is_ascii_alphabetic() && !c.is_ascii_hexdigit())
}

/// Scan source for webpack signatures.
#[must_use]
pub fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    let mut out = Vec::new();
    let fp = || file_path.map(str::to_string);

    // webpack_jsonp_global — the build's unique chunk-loading global name.
    let mut seen_global = false;
    for caps in JSONP_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let name = caps.get(1).map_or("", |m| m.as_str());
        if seen_global {
            break; // one per bundle is enough
        }
        seen_global = true;
        out.push(
            AnalyzerMatch::new(
                "webpack_jsonp_global",
                name.to_string(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                fp(),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info)
            .with_metadata(serde_json::json!({ "unique_name": name })),
        );
    }

    // webpack_runtime — presence of the require/registry runtime.
    if let Some(m) = RUNTIME_RE.find(source) {
        out.push(
            AnalyzerMatch::new(
                "webpack_runtime",
                m.as_str().to_string(),
                PositionRange::from_byte_range(source, m.start(), m.end()),
                file_type,
                fp(),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info),
        );
    }

    // webpack_chunk_name — mine the `.u`/`.miniCssF` chunk-filename map.
    for fn_caps in CHUNK_FN_RE.captures_iter(source) {
        let kind = fn_caps.get(1).map_or("", |m| m.as_str());
        let tail = fn_caps.get(2).map_or("", |m| m.as_str());
        let tail_start = fn_caps.get(2).map_or(0, |m| m.start());
        for pair in CHUNK_PAIR_RE.captures_iter(tail) {
            let id = pair
                .get(1)
                .or_else(|| pair.get(2))
                .map_or("", |m| m.as_str())
                .to_string();
            let name = pair.get(3).map_or("", |m| m.as_str()).to_string();
            if !looks_like_chunk_name(&name) {
                continue; // skip content-hash maps
            }
            let whole = pair.get(0).unwrap();
            let abs = tail_start + whole.start();
            out.push(
                AnalyzerMatch::new(
                    "webpack_chunk_name",
                    name.clone(),
                    PositionRange::from_byte_range(source, abs, abs + whole.as_str().len()),
                    file_type,
                    fp(),
                )
                .with_provenance(Provenance::Other)
                .with_severity(Severity::Info)
                .with_metadata(serde_json::json!({ "chunk_id": id, "chunk_name": name, "map": kind })),
            );
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_jsonp_global_and_runtime() {
        let src =
            r#"(self.webpackChunk_N_E=self.webpackChunk_N_E||[]).push([[1],{2:function(e,t,__webpack_require__){}}]);"#;
        let m = analyze(src, Some("t.js"), FileType::Js);
        let g = m
            .iter()
            .find(|x| x.match_kind == "webpack_jsonp_global")
            .expect("global");
        assert_eq!(g.match_value, "webpackChunk_N_E");
        assert!(m.iter().any(|x| x.match_kind == "webpack_runtime"));
    }

    #[test]
    fn mines_chunk_name_map_skipping_hashes() {
        // `.u` maps chunkId -> name, then chunkId -> content hash. Only names kept.
        let src = r#"r.u=e=>"static/chunks/"+({26:"vendors-node_modules",404:"pages/admin"}[e]||e)+"."+{26:"a1b2c3d4",404:"deadbeef"}[e]+".js";"#;
        let m = analyze(src, Some("t.js"), FileType::Js);
        let names: Vec<_> = m
            .iter()
            .filter(|x| x.match_kind == "webpack_chunk_name")
            .map(|x| x.match_value.as_str())
            .collect();
        assert!(names.contains(&"vendors-node_modules"), "got: {names:?}");
        assert!(names.contains(&"pages/admin"), "got: {names:?}");
        // content hashes (pure hex) must be skipped
        assert!(!names.iter().any(|n| *n == "a1b2c3d4" || *n == "deadbeef"));
    }

    #[test]
    fn empty_on_non_webpack() {
        assert!(analyze("const x = 1;", None, FileType::Js).is_empty());
    }
}
