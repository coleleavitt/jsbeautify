//! Module Federation recon analyzer. Surfaces MF runtime globals, remote entry
//! URLs (cross-origin attack surface), exposed module maps, shared dependency
//! version tables, and mf-manifest/mf-stats document shapes in production
//! bundles.
//!
//! Signatures that survive minification in production bundles; emits AnalyzerMatch.
//!
//! Literal signatures confirmed against the cloned current source at
//! /home/cole/RustProjects/forks/frameworks/module-federation :
//!   - `__FEDERATION__.__INSTANCES__`  -> packages/runtime/src/utils.ts:18, index.ts:119
//!   - `<require>.federation = { ... }` -> packages/enhanced/src/lib/container/runtime/utils.ts:41
//!     (getFederationGlobalScope returns `${require||'__webpack_require__'}.federation`)
//!   - `remoteInfos` items `{alias,name,entry,shareScope,externalType}`
//!     -> packages/enhanced/src/lib/container/runtime/getFederationGlobal.ts:25-51
//!   - `exposes` array `{id,name,path}` (ManifestExpose) -> packages/sdk/src/types/manifest.ts:39-43;
//!     container `var moduleMap = {...}` -> packages/enhanced/src/lib/container/ContainerEntryModule.ts:279
//!   - `shared` entries `{id,name,version,singleton,requiredVersion}` (ManifestShared)
//!     -> packages/sdk/src/types/manifest.ts:9-21
//!   - manifest/stats top-level `{id,name,metaData,shared,remotes,exposes}`
//!     -> packages/sdk/src/types/manifest.ts:46-54, packages/sdk/src/types/stats.ts:135-142
use super::match_kind::{AnalyzerMatch, FileType, PositionRange, Provenance};
use super::regex_bank::Severity;
use regex::Regex;

/// Scan source for Module Federation signatures.
#[must_use]
pub fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    let mut out = Vec::new();
    lazy_static::lazy_static! {
        // mf_runtime: the MF runtime global. `__FEDERATION__[.__INSTANCES__]`
        // (runtime/src/utils.ts) or the bundler-emitted `<require>.federation = {`
        // assignment (enhanced .../runtime/utils.ts:41).
        static ref MF_RUNTIME_RE: Regex = Regex::new(
            r#"(?:__FEDERATION__(?:\.__INSTANCES__)?|\.federation\s*=\s*\{)"#
        ).unwrap();

        // mf_remote: a remote descriptor pairing a `name` with an `entry` URL,
        // as emitted into remoteInfos / initOptions.remotes
        // (getFederationGlobal.ts builds `{alias,name,entry,shareScope,...}`).
        // Keys may be JSON-quoted or bare object-literal keys. Captures
        // (1)=remote name, (2)=remoteEntry URL.
        // The entry is either an absolute URL or a relative `remoteEntry`-shaped
        // path (`/remotes/app/remoteEntry.js`, `./remoteEntry.js`). Requiring one
        // of those shapes (rather than any string) keeps the `{name,entry}`
        // adjacency from matching ordinary object literals like `{name,entry:"main"}`.
        static ref MF_REMOTE_RE: Regex = Regex::new(
            r#"["']?name["']?\s*:\s*["']([^"']+)["']\s*,\s*["']?entry["']?\s*:\s*["'](https?://[^"']+|[^"']*remoteEntry[^"']*)["']"#
        ).unwrap();

        // mf_expose: an `exposes` array whose first object carries a `./`-prefixed
        // `path` (ManifestExpose {id,name,path}). Captures (1)=expose path.
        static ref MF_EXPOSE_RE: Regex = Regex::new(
            r#"["']exposes["']\s*:\s*\[\s*\{[^}]*["']path["']\s*:\s*["'](\./[^"']+)["']"#
        ).unwrap();

        // mf_shared: a shared dependency version-table entry. Anchored by the
        // MF-specific `singleton` key following name+version (ManifestShared).
        // Captures (1)=package name, (2)=resolved version.
        static ref MF_SHARED_RE: Regex = Regex::new(
            r#"["']name["']\s*:\s*["']([^"']+)["']\s*,\s*["']version["']\s*:\s*["']([^"']+)["']\s*,\s*["']singleton["']"#
        ).unwrap();

        // mf_manifest: the mf-manifest.json / mf-stats.json document head
        // `{id,name,metaData,...}`. The camelCase `metaData` discriminates the
        // MF document from generic {id,name} objects. Captures (1)=id, (2)=name.
        static ref MF_MANIFEST_RE: Regex = Regex::new(
            r#"["']id["']\s*:\s*["']([^"']+)["']\s*,\s*["']name["']\s*:\s*["']([^"']+)["']\s*,\s*["']metaData["']"#
        ).unwrap();
    }

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
