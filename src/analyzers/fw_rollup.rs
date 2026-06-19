//! Rollup recon analyzer. Fingerprints Rollup-generated bundles by their
//! finaliser preambles (UMD / System.register) and interop helper bodies.
//!
//! Signatures that survive minification in production bundles; emits `AnalyzerMatch`.
//!
//! Confirmed against the cloned Rollup source at
//! `/home/cole/RustProjects/forks/frameworks/rollup`:
//!   - `src/utils/interopHelpers.ts`
//!       * `getDefaultStatic` (line ~205) -> `e&&e.__esModule?e.default:e`
//!       * `MERGE_NAMESPACES_VARIABLE` (line 10) + generator (line ~178) -> `_mergeNamespaces`
//!       * `DOCUMENT_CURRENT_SCRIPT` generator (line 75) ->
//!         `typeof document!=='undefined'?document.currentScript:null`
//!   - `src/finalisers/umd.ts` (lines ~175-182) -> the UMD/CJS/AMD preamble ternary
//!   - `src/finalisers/system.ts` (line ~52) -> `System.register(<name>[<deps>],(function(exports,module){...})`
//!   - `src/utils/options/normalizeOutputOptions.ts:54` -> default `chunkFileNames: '[name]-[hash].js'`
//!
//! NOTE: `getDefaultExportFromCjs` and `commonjsGlobal` are emitted by
//! `@rollup/plugin-commonjs` (not Rollup core; grep of the core clone is empty),
//! but they are ubiquitous in real Rollup bundles, so they fold into the
//! `rollup_interop_helper` kind for recon completeness.
use super::match_kind::{AnalyzerMatch, FileType, PositionRange, Provenance};
use super::regex_bank::Severity;
use regex::Regex;
use std::sync::LazyLock;

static INTEROP_HELPERS: LazyLock<Vec<(&'static str, Regex)>> = LazyLock::new(|| {
    vec![
        ("interop_default", Regex::new(r"\be\s*&&\s*e\.__esModule\s*\?").unwrap()),
        (
            "merge_namespaces",
            Regex::new(r"function\s+_mergeNamespaces\s*\(").unwrap(),
        ),
        (
            "document_current_script",
            Regex::new(r#"typeof\s+document\s*!==\s*['"]undefined['"]\s*\?\s*document\.currentScript\s*:\s*null"#)
                .unwrap(),
        ),
        (
            "get_default_export_from_cjs",
            Regex::new(r"\bfunction\s+getDefaultExportFromCjs\b").unwrap(),
        ),
        (
            "commonjs_global",
            Regex::new(r"\bcommonjsGlobal\s*=\s*typeof\s+globalThis").unwrap(),
        ),
    ]
});
static UMD_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?s)typeof\s+exports\s*===\s*['"]object['"]\s*&&\s*typeof\s+module\s*!==\s*['"]undefined['"].{0,200}?typeof\s+define\s*===\s*['"]function['"]\s*&&\s*define\.amd"#,
    )
    .unwrap()
});
static SYSTEM_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"System\.register\(\s*[^\[]*\[([^\]]*)\]").unwrap());
static CHUNK_FACADE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"["'`(/]([A-Za-z0-9_$.\-]+-[0-9A-Za-z_$]{8}\.m?js)["'`)]"#).unwrap());
static FACADE_PATH_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"-[0-9A-Za-z_$]{8}\.m?js$").unwrap());

/// Scan source for Rollup bundle signatures.
///
/// # Panics
///
/// Panics only if a compile-time regex pattern is invalid.
#[must_use]
pub fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    let mut out = Vec::new();

    // --- Interop helpers (kind: rollup_interop_helper) ---
    for (label, re) in INTEROP_HELPERS.iter() {
        for caps in re.captures_iter(source) {
            let whole = caps.get(0).unwrap();
            out.push(
                AnalyzerMatch::new(
                    "rollup_interop_helper",
                    whole.as_str().to_string(),
                    PositionRange::from_byte_range(source, whole.start(), whole.end()),
                    file_type,
                    file_path.map(str::to_string),
                )
                .with_provenance(Provenance::Other)
                .with_severity(Severity::Info)
                .with_metadata(serde_json::json!({ "helper": *label })),
            );
        }
    }

    // --- UMD finaliser (kind: rollup_umd) ---
    for caps in UMD_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        out.push(
            AnalyzerMatch::new(
                "rollup_umd",
                whole.as_str().to_string(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                file_path.map(str::to_string),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info),
        );
    }

    // --- System.register finaliser (kind: rollup_system_register) ---
    for caps in SYSTEM_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let deps_raw = caps.get(1).map_or("", |m| m.as_str());
        let deps: Vec<String> = deps_raw
            .split(',')
            .map(|d| d.trim().trim_matches(|c| c == '\'' || c == '"' || c == '`').to_string())
            .filter(|d| !d.is_empty())
            .collect();
        out.push(
            AnalyzerMatch::new(
                "rollup_system_register",
                deps_raw.to_string(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                file_path.map(str::to_string),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info)
            .with_metadata(serde_json::json!({ "dependencies": deps })),
        );
    }

    // --- ESM chunk facade references (kind: rollup_chunk_facade) ---
    // The bare `name-<8hex>.js` shape also matches webpack/Vite hashed chunk
    // filenames, so only treat in-source references as Rollup facades when a
    // real Rollup signal (interop helper / UMD / System.register) is present —
    // otherwise this is a pure false positive on non-Rollup bundles.
    let has_rollup_signal = !out.is_empty();
    if has_rollup_signal {
        for caps in CHUNK_FACADE_RE.captures_iter(source) {
            let whole = caps.get(0).unwrap();
            let name = caps.get(1).map_or("", |m| m.as_str()).to_string();
            out.push(
                AnalyzerMatch::new(
                    "rollup_chunk_facade",
                    name,
                    PositionRange::from_byte_range(source, whole.start(), whole.end()),
                    file_type,
                    file_path.map(str::to_string),
                )
                .with_provenance(Provenance::Other)
                .with_severity(Severity::Info)
                .with_metadata(serde_json::json!({ "detected_from": "reference" })),
            );
        }
    }

    // The file path itself may be a Rollup chunk facade.
    if let Some(path) = file_path
        && FACADE_PATH_RE.is_match(path)
    {
        out.push(
            AnalyzerMatch::new(
                "rollup_chunk_facade",
                path.to_string(),
                PositionRange::from_byte_range(source, 0, 0),
                file_type,
                Some(path.to_string()),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info)
            .with_metadata(serde_json::json!({
                "detected_from": "file_path",
                // The `-<8hex>.js` filename shape is shared by webpack/Vite/
                // Rollup output, so a filename alone (no in-source Rollup
                // signal) is only weak evidence of Rollup specifically.
                "confidence": "low",
            })),
        );
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_interop_helper() {
        // Minified Rollup core interop block (see src/utils/interopHelpers.ts).
        let src = r"var _documentCurrentScript=typeof document!=='undefined'?document.currentScript:null;function _interopDefault(e){return e&&e.__esModule?e.default:e}function _mergeNamespaces(n,m){return n}";
        let m = analyze(src, Some("bundle.js"), FileType::Js);
        assert!(m.iter().any(|x| x.match_kind == "rollup_interop_helper"));
        // all three sub-signatures should be present
        let helpers: Vec<_> = m
            .iter()
            .filter_map(|x| {
                x.metadata
                    .as_ref()
                    .and_then(|v| v.get("helper"))
                    .and_then(|h| h.as_str())
            })
            .collect();
        assert!(helpers.contains(&"interop_default"));
        assert!(helpers.contains(&"merge_namespaces"));
        assert!(helpers.contains(&"document_current_script"));
    }

    #[test]
    fn detects_umd_finaliser() {
        // Real Rollup UMD preamble shape (see src/finalisers/umd.ts).
        let src = r"(function (global, factory) {
typeof exports === 'object' && typeof module !== 'undefined' ? factory(exports) :
typeof define === 'function' && define.amd ? define(['exports'], factory) :
(global = typeof globalThis !== 'undefined' ? globalThis : global || self, factory(global.MyLib = {}));
})(this, (function (exports) { 'use strict'; }));";
        let m = analyze(src, Some("my-lib.umd.js"), FileType::Js);
        assert!(m.iter().any(|x| x.match_kind == "rollup_umd"));
    }

    #[test]
    fn detects_system_register_and_chunk_facade() {
        // Real Rollup System finaliser shape (see src/finalisers/system.ts).
        let src = r"System.register('main', ['./vendor-a1b2c3d4.js', './polyfills.js'], (function (exports, module) {
'use strict';
return { setters: [], execute: function () {} };
}));";
        let m = analyze(src, Some("app.system.js"), FileType::Js);

        let sys = m.iter().find(|x| x.match_kind == "rollup_system_register").unwrap();
        let deps = sys.metadata.as_ref().unwrap().get("dependencies").unwrap();
        assert_eq!(deps[0], "./vendor-a1b2c3d4.js");
        assert_eq!(deps[1], "./polyfills.js");

        // the `vendor-a1b2c3d4.js` reference is a Rollup chunk facade
        assert!(
            m.iter()
                .any(|x| x.match_kind == "rollup_chunk_facade" && x.match_value == "vendor-a1b2c3d4.js")
        );
    }
}
