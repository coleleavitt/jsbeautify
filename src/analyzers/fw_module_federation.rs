//! Module Federation recon analyzer.
//!
//! Surfaces MF runtime globals, remote entry URLs, exposed module maps, shared
//! dependency version tables, and mf-manifest/mf-stats document shapes in
//! production bundles.
//!
//! Signatures that survive minification in production bundles; emits `AnalyzerMatch`.
//!
//! Literal signatures confirmed against the cloned current source at
//! /home/cole/RustProjects/forks/frameworks/module-federation :
//! - `__FEDERATION__.__INSTANCES__` -> packages/runtime/src/utils.ts:18, index.ts:119
//! - `<require>.federation = { ... }` -> packages/enhanced/src/lib/container/runtime/utils.ts:41
//! - `remoteInfos` items `{alias,name,entry,shareScope,externalType}` -> packages/enhanced/src/lib/container/runtime/getFederationGlobal.ts:25-51
//! - `exposes` array `{id,name,path}` (`ManifestExpose`) -> packages/sdk/src/types/manifest.ts:39-43
//! - `shared` entries `{id,name,version,singleton,requiredVersion}` (`ManifestShared`) -> packages/sdk/src/types/manifest.ts:9-21
//! - manifest/stats top-level `{id,name,metaData,shared,remotes,exposes}` -> packages/sdk/src/types/manifest.ts:46-54
use super::match_kind::{AnalyzerMatch, FileType, PositionRange, Provenance};
use super::regex_bank::Severity;
use regex::Regex;
use std::sync::LazyLock;

static MF_RUNTIME_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:__FEDERATION__(?:\.__INSTANCES__)?|\.federation\s*=\s*\{)").unwrap());
static MF_REMOTE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"["']?name["']?\s*:\s*["']([^"']+)["']\s*,\s*["']?entry["']?\s*:\s*["'](https?://[^"']+|[^"']*remoteEntry[^"']*)["']"#,
    )
    .unwrap()
});
static MF_EXPOSE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"["']exposes["']\s*:\s*\[\s*\{[^}]*["']path["']\s*:\s*["'](\./[^"']+)["']"#).unwrap()
});
static MF_SHARED_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"["']name["']\s*:\s*["']([^"']+)["']\s*,\s*["']version["']\s*:\s*["']([^"']+)["']\s*,\s*["']singleton["']"#,
    )
    .unwrap()
});
static MF_MANIFEST_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"["']id["']\s*:\s*["']([^"']+)["']\s*,\s*["']name["']\s*:\s*["']([^"']+)["']\s*,\s*["']metaData["']"#)
        .unwrap()
});

/// Scan source for Module Federation signatures.
///
/// # Panics
///
/// Panics only if a compile-time regex pattern is invalid.
#[must_use]
pub fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    let mut out = Vec::new();

    // mf_runtime — runtime global presence.
    for caps in MF_RUNTIME_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        out.push(
            AnalyzerMatch::new(
                "mf_runtime",
                whole.as_str().to_string(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                file_path.map(str::to_string),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info),
        );
    }

    // mf_remote — remote name + cross-origin remoteEntry URL.
    for caps in MF_REMOTE_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let name = caps.get(1).map_or("", |m| m.as_str()).to_string();
        let entry = caps.get(2).map_or("", |m| m.as_str()).to_string();
        out.push(
            AnalyzerMatch::new(
                "mf_remote",
                entry.clone(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                file_path.map(str::to_string),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Medium)
            .with_metadata(serde_json::json!({
                "remote_name": name,
                "remote_entry": entry,
            })),
        );
    }

    // mf_expose — exposed module path.
    for caps in MF_EXPOSE_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let path = caps.get(1).map_or("", |m| m.as_str()).to_string();
        out.push(
            AnalyzerMatch::new(
                "mf_expose",
                path.clone(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                file_path.map(str::to_string),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info)
            .with_metadata(serde_json::json!({ "expose_path": path })),
        );
    }

    // mf_shared — shared dependency version table.
    for caps in MF_SHARED_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let pkg = caps.get(1).map_or("", |m| m.as_str()).to_string();
        let version = caps.get(2).map_or("", |m| m.as_str()).to_string();
        out.push(
            AnalyzerMatch::new(
                "mf_shared",
                format!("{pkg}@{version}"),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                file_path.map(str::to_string),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info)
            .with_metadata(serde_json::json!({
                "package": pkg,
                "version": version,
            })),
        );
    }

    // mf_manifest — mf-manifest.json / mf-stats.json document shape.
    for caps in MF_MANIFEST_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let id = caps.get(1).map_or("", |m| m.as_str()).to_string();
        let name = caps.get(2).map_or("", |m| m.as_str()).to_string();
        out.push(
            AnalyzerMatch::new(
                "mf_manifest",
                name.clone(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                file_path.map(str::to_string),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info)
            .with_metadata(serde_json::json!({
                "manifest_id": id,
                "manifest_name": name,
            })),
        );
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Minified-shape snippet mirroring getFederationGlobal.ts output: the
    // `<require>.federation = { ... }` assignment plus a remoteInfos table
    // whose items are `{alias,name,entry,shareScope,externalType}`.
    #[test]
    fn detects_core_signature() {
        let src = r#"if(!__webpack_require__.federation){__webpack_require__.federation = {initOptions:{name:"host",remotes:[]},bundlerRuntimeOptions:{remotes:{remoteInfos:{"app1":[{"alias":"","name":"app1","entry":"https://cdn.example.com/app1/remoteEntry.js","shareScope":"default","externalType":"script"}]}}}}};}"#;
        let m = analyze(src, Some("t.js"), FileType::Js);
        assert!(m.iter().any(|x| x.match_kind == "mf_runtime"));
    }

    #[test]
    fn captures_remote_entry_url_as_medium() {
        let src = r#"{"alias":"","name":"checkout","entry":"https://evil.cdn.test/checkout/remoteEntry.js","shareScope":"default","externalType":"script"}"#;
        let m = analyze(src, None, FileType::Js);
        let remote = m.iter().find(|x| x.match_kind == "mf_remote").expect("mf_remote match");
        assert_eq!(remote.match_value, "https://evil.cdn.test/checkout/remoteEntry.js");
        assert_eq!(remote.severity, Severity::Medium);
    }

    // Minified-shape snippet mirroring the mf-manifest.json document:
    // {id,name,metaData,shared:[{...,singleton}],remotes,exposes:[{...,path}]}.
    #[test]
    fn detects_manifest_shared_and_expose() {
        let src = r#"{"id":"app1","name":"app1","metaData":{"name":"app1","type":"app"},"shared":[{"id":"app1:react","name":"react","version":"18.2.0","singleton":true,"requiredVersion":"^18.2.0","assets":{"js":{"sync":[],"async":[]}}}],"remotes":[],"exposes":[{"id":"app1:Button","name":"Button","path":"./Button","assets":{"js":{"sync":["button.js"],"async":[]}}}]}"#;
        let kinds: Vec<_> = analyze(src, Some("mf-manifest.json"), FileType::Js)
            .into_iter()
            .map(|x| x.match_kind)
            .collect();
        assert!(kinds.iter().any(|k| k == "mf_manifest"));
        assert!(kinds.iter().any(|k| k == "mf_shared"));
        assert!(kinds.iter().any(|k| k == "mf_expose"));
    }
}
