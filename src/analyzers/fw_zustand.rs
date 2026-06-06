//! Zustand recon analyzer. Fingerprints Zustand v5 stores + middleware in minified bundles.
//!
//! Signatures that survive minification in production bundles; emits AnalyzerMatch.
//!
//! Literal signatures confirmed against the cloned upstream source
//! (`/home/cole/RustProjects/forks/frameworks/zustand`):
//!   - core api 4-key shape  -> src/vanilla.ts:94  `const api = { setState, getState, getInitialState, subscribe }`
//!   - useSyncExternalStore  -> src/react.ts:30, src/traditional.ts:36
//!   - persist beacon/opts   -> src/middleware/persist.ts:212 + :69/:190-193 (name/storage/partialize/version/merge)
//!   - SSR error string      -> src/middleware/ssrSafe.ts:20  `Cannot set state of Zustand store in SSR`
//!   - devtools error string -> src/middleware/devtools.ts:314 `[zustand devtools middleware] Unsupported action format`
//!   - immer produce shape   -> src/middleware/immer.ts:79  `typeof updater === 'function' ? produce(updater ...)`
//!
//! NOTE: the `zustand/persist` import path is erased by bundlers, so detection keys on the
//! runtime error beacon + the persist options object rather than the import specifier.

use super::match_kind::{AnalyzerMatch, FileType, PositionRange, Provenance};
use super::regex_bank::Severity;
use regex::Regex;

/// Scan source for Zustand (v5) store + middleware signatures.
#[must_use]
pub fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    let mut out = Vec::new();

    lazy_static::lazy_static! {
        // Core StoreApi object literal: the canonical v5 4-key shape, in source order.
        // Matches both shorthand `{ setState, getState, getInitialState, subscribe }` and
        // minified `{setState:a,getState:b,getInitialState:c,subscribe:d}`. The keys are
        // public API property names and survive minification; `getInitialState` is v5-specific.
        static ref STORE_API_RE: Regex = Regex::new(
            r#"\{\s*setState\s*[:,][^{}]*?getState\s*[:,][^{}]*?getInitialState\s*[:,][^{}]*?subscribe\s*[:,}]"#
        ).unwrap();
        // React subscription hook (covers `useSyncExternalStore` and `useSyncExternalStoreWithSelector`).
        static ref USE_SYNC_RE: Regex = Regex::new(r#"useSyncExternalStore"#).unwrap();

        // Persist middleware runtime beacon (gate; the import path is erased).
        static ref PERSIST_BEACON_RE: Regex =
            Regex::new(r#"\[zustand persist middleware\]"#).unwrap();
        // persist options: localStorage/sessionStorage key. `name:"..."` immediately followed by
        // a persist-specific sibling option (the canonical/default serialized order).
        static ref PERSIST_NAME_A_RE: Regex = Regex::new(
            r#"name\s*:\s*["']([^"'\\]{1,128})["']\s*,\s*(?:storage|getStorage|partialize|version|merge|migrate|onRehydrateStorage|skipHydration|serialize|deserialize)\b"#
        ).unwrap();
        // ...or a persist-specific option followed (within a small, brace-free span) by `name:"..."`.
        static ref PERSIST_NAME_B_RE: Regex = Regex::new(
            r#"(?:storage|getStorage|partialize|version|merge|migrate|onRehydrateStorage|skipHydration|serialize|deserialize)\s*:[^{}]{0,200}?name\s*:\s*["']([^"'\\]{1,128})["']"#
        ).unwrap();

        // ssrSafe middleware throw string.
        static ref SSR_RE: Regex =
            Regex::new(r#"Cannot set state of Zustand store in SSR"#).unwrap();
        // devtools middleware console.error fingerprint.
        static ref DEVTOOLS_RE: Regex =
            Regex::new(r#"\[zustand devtools middleware\] Unsupported action format"#).unwrap();
        // immer middleware: `typeof x === 'function' ? produce(x) : x` in either operand order,
        // tolerating the bundler interop wrapper `(0,ns.produce)(...)`.
        static ref IMMER_RE: Regex = Regex::new(
            r#"(?:typeof\s+[\w$.]+\s*===?\s*["']function["']|["']function["']\s*===?\s*typeof\s+[\w$.]+)\s*\?[^?{};]{0,48}produce\b"#
        ).unwrap();
    }

    // --- zustand_store: 4-key StoreApi shape co-occurring with useSyncExternalStore ---
    if USE_SYNC_RE.is_match(source) {
        for whole in STORE_API_RE.find_iter(source) {
            out.push(
                AnalyzerMatch::new(
                    "zustand_store",
                    whole.as_str().to_string(),
                    PositionRange::from_byte_range(source, whole.start(), whole.end()),
                    file_type,
                    file_path.map(str::to_string),
                )
                .with_provenance(Provenance::Other)
                .with_severity(Severity::Info),
            );
        }
    }

    // --- zustand_persist_key: localStorage key (secrets live here) ---
    // Only mine the persist options object once the runtime beacon confirms persist is bundled.
    if PERSIST_BEACON_RE.is_match(source) {
        let mut seen_keys = std::collections::HashSet::new();
        for re in [&*PERSIST_NAME_A_RE, &*PERSIST_NAME_B_RE] {
            for caps in re.captures_iter(source) {
                let whole = caps.get(0).unwrap();
                let val = caps.get(1).map_or("", |m| m.as_str()).to_string();
                if val.is_empty() || !seen_keys.insert(val.clone()) {
                    continue;
                }
                out.push(
                    AnalyzerMatch::new(
                        "zustand_persist_key",
                        val.clone(),
                        PositionRange::from_byte_range(source, whole.start(), whole.end()),
                        file_type,
                        file_path.map(str::to_string),
                    )
                    .with_provenance(Provenance::Other)
                    .with_severity(Severity::Medium)
                    .with_metadata(serde_json::json!({
                        "storage_key": val,
                        "middleware": "zustand/persist",
                        "note": "localStorage/sessionStorage key — inspect persisted state for tokens/secrets",
                    })),
                );
            }
        }
    }

    // --- zustand_ssr: ssrSafe middleware throw string ---
    for whole in SSR_RE.find_iter(source) {
        out.push(
            AnalyzerMatch::new(
                "zustand_ssr",
                whole.as_str().to_string(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                file_path.map(str::to_string),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info),
        );
    }

    // --- zustand_devtools: redux-devtools middleware error fingerprint ---
    for whole in DEVTOOLS_RE.find_iter(source) {
        out.push(
            AnalyzerMatch::new(
                "zustand_devtools",
                whole.as_str().to_string(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                file_path.map(str::to_string),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info),
        );
    }

    // --- zustand_immer: produce()-wrapped setState shape ---
    for whole in IMMER_RE.find_iter(source) {
        out.push(
            AnalyzerMatch::new(
                "zustand_immer",
                whole.as_str().to_string(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                file_path.map(str::to_string),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info),
        );
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_core_signature() {
        // Minified shape of src/vanilla.ts:94 + the React subscription hook (src/react.ts:30).
        let src = r#"const o={setState:s,getState:t,getInitialState:n,subscribe:c};const u=e.useSyncExternalStore(o.subscribe,o.getState,o.getInitialState);"#;
        let m = analyze(src, Some("t.js"), FileType::Js);
        assert!(m.iter().any(|x| x.match_kind == "zustand_store"));
    }

    #[test]
    fn store_shape_requires_use_sync_external_store() {
        // The 4-key object alone (no useSyncExternalStore) must not fire zustand_store.
        let src = r#"const o={setState:s,getState:t,getInitialState:n,subscribe:c};"#;
        let m = analyze(src, None, FileType::Js);
        assert!(!m.iter().any(|x| x.match_kind == "zustand_store"));
    }

    #[test]
    fn detects_persist_key_with_metadata() {
        // Beacon (persist.ts:212) gates extraction of the persist options name (persist.ts:69/190).
        let src = r#"throw new Error("[zustand persist middleware] Unable to update item");const c=p(f,{name:"auth-store",storage:g,version:1});"#;
        let m = analyze(src, Some("t.js"), FileType::Js);
        let hit = m
            .iter()
            .find(|x| x.match_kind == "zustand_persist_key")
            .expect("expected zustand_persist_key match");
        assert_eq!(hit.match_value, "auth-store");
        assert_eq!(hit.severity, Severity::Medium);
        let meta = hit.metadata.as_ref().expect("metadata present");
        assert_eq!(meta["storage_key"], "auth-store");
    }

    #[test]
    fn detects_ssr_devtools_and_immer() {
        // ssrSafe (ssrSafe.ts:20), devtools (devtools.ts:314), immer (immer.ts:79) minified.
        let src = r#"function x(){if(!a)return s(e);throw new Error("Cannot set state of Zustand store in SSR")}
console.error("[zustand devtools middleware] Unsupported action format");
r.setState=(e,t,...n)=>{const o="function"==typeof e?(0,a.produce)(e):e;return s(o,t,...n)};"#;
        let kinds: Vec<_> = analyze(src, None, FileType::Js)
            .into_iter()
            .map(|x| x.match_kind)
            .collect();
        assert!(kinds.iter().any(|k| k == "zustand_ssr"));
        assert!(kinds.iter().any(|k| k == "zustand_devtools"));
        assert!(kinds.iter().any(|k| k == "zustand_immer"));
    }
}
