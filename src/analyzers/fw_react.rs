//! React recon analyzer. Detects React 19 / React Compiler runtime fingerprints
//! that survive minification in production bundles (the memo-cache sentinel +
//! `_c(N)` cache prelude, the `$$typeof` element-symbol factory tags, and the
//! versioned `__*_INTERNALS*` / `__COMPILER_RUNTIME` export names).
//!
//! Signatures that survive minification in production bundles; emits AnalyzerMatch.
//!
//! Signatures confirmed against the cloned React source (react @ 19.3.0,
//! `ReactVersions.js` L?: `const ReactVersion = '19.3.0';`):
//!   - memo-cache sentinel + `const $ = _c(N)` prelude + `$[i] === Symbol.for(
//!     "react.memo_cache_sentinel")` guard chain:
//!       compiler/packages/babel-plugin-react-compiler/src/__tests__/fixtures/
//!         compiler/allow-global-mutation-in-effect-indirect-usecallback.expect.md
//!         (`import { c as _c } from "react/compiler-runtime";` / `const $ = _c(6);`
//!          / `if ($[0] === Symbol.for("react.memo_cache_sentinel")) {`)
//!     The cache fn is `react/compiler-runtime`'s `c`, aliased `_c`:
//!       packages/react/src/ReactCompilerRuntime.js
//!         (`export {useMemoCache as c} from './ReactHooks';`)
//!   - `$$typeof` element tagging via the element symbols:
//!       packages/react/src/jsx/ReactJSXElement.js
//!         (`REACT_ELEMENT_TYPE` == `Symbol.for('react.transitional.element')`,
//!          `element = { $$typeof: REACT_ELEMENT_TYPE, type, key, props, ... }`)
//!       packages/react/src/ReactForwardRef.js  (`$$typeof: REACT_FORWARD_REF_TYPE`)
//!       packages/react/src/ReactMemo.js         (`$$typeof: REACT_MEMO_TYPE`)
//!       packages/react/src/ReactLazy.js         (`$$typeof: REACT_LAZY_TYPE`)
//!       packages/react/src/ReactContext.js      (`$$typeof: REACT_CONTEXT_TYPE`,
//!                                                 `REACT_CONSUMER_TYPE`)
//!       (`react.transitional.element` / `react.portal` literals also verified in
//!        packages/react/src/jsx/ReactJSXElement.js L165 and
//!        packages/react-reconciler/src/__tests__/ReactPersistent-test.js L31)
//!   - runtime internals export names / compiler-runtime re-export:
//!       packages/react/src/ReactClient.js L110-111
//!         (`ReactSharedInternals as
//!            __CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE,`
//!          `ReactCompilerRuntime as __COMPILER_RUNTIME,`)
//!       legacy `__SECRET_INTERNALS_DO_NOT_USE_OR_YOU_WILL_BE_FIRED` is the React
//!       <= 18 internals name (not present in the 19.3.0 clone) — used purely to
//!       discriminate the version era.

use super::match_kind::{AnalyzerMatch, FileType, PositionRange, Provenance};
use super::regex_bank::Severity;
use lazy_static::lazy_static;
use regex::Regex;

lazy_static! {
    /// Canonical React-Compiler cache prelude: `const $ = _c(6);`. `_c` is the
    /// import alias for `react/compiler-runtime`'s `c` (== `useMemoCache`). One
    /// prelude per compiled function; capture group 1 is the slot count N.
    /// Confirmed: allow-global-mutation-in-effect-indirect-usecallback.expect.md.
    static ref COMPILER_PRELUDE_RE: Regex = Regex::new(r#"\b_c\(\s*(\d+)\s*\)"#).unwrap();

    /// The memo-cache guard chain `X[i] === Symbol.for("react.memo_cache_sentinel")`.
    /// The sentinel string literal always survives minification even when the
    /// `_c` alias is mangled. Group 1 = cache var, group 2 = slot index.
    static ref SENTINEL_CHAIN_RE: Regex = Regex::new(
        r#"([A-Za-z_$][0-9A-Za-z_$]*)\[(\d+)\]\s*===\s*Symbol\.for\(\s*["']react\.memo_cache_sentinel["']\s*\)"#,
    )
    .unwrap();

    /// Element/factory `$$typeof` symbols created via `Symbol.for("react.<kind>")`.
    /// Deliberately excludes `react.memo_cache_sentinel` (the trailing quote after
    /// `memo` prevents a partial match). Group 1 = the `react.<kind>` symbol name.
    static ref ELEMENT_SYMBOL_RE: Regex = Regex::new(
        r#"Symbol\.for\(\s*["'](react\.(?:transitional\.element|forward_ref|memo|lazy|context|consumer|portal|fragment|suspense|profiler|strict_mode|provider))["']\s*\)"#,
    )
    .unwrap();

    /// React shared-internals / compiler-runtime export names. The modern client
    /// name (`__CLIENT_INTERNALS_...`) marks React 19+, the legacy fired name
    /// marks <= 18, and `__COMPILER_RUNTIME` flags compiler-emitted bundles.
    /// ReactClient.js L110-111. Group 1 = the matched internals identifier.
    static ref RUNTIME_FP_RE: Regex = Regex::new(
        r#"(__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE|__SECRET_INTERNALS_DO_NOT_USE_OR_YOU_WILL_BE_FIRED|__COMPILER_RUNTIME)"#,
    )
    .unwrap();
}

/// Scan source for React signatures.
#[must_use]
pub fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    let mut out = Vec::new();
    let fp = || file_path.map(str::to_string);

    // react_compiler_fn — memo-cache sentinel + `_c(N)` prelude / guard chain.
    // Gate on the sentinel string so a stray `_c(3)` call can't false-fire.
    if source.contains("react.memo_cache_sentinel") {
        let mut emitted_prelude = false;
        for caps in COMPILER_PRELUDE_RE.captures_iter(source) {
            let whole = caps.get(0).unwrap();
            let slot_count: u32 = caps.get(1).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
            emitted_prelude = true;
            out.push(
                AnalyzerMatch::new(
                    "react_compiler_fn",
                    whole.as_str(),
                    PositionRange::from_byte_range(source, whole.start(), whole.end()),
                    file_type,
                    fp(),
                )
                .with_provenance(Provenance::Other)
                .with_severity(Severity::Info)
                .with_metadata(serde_json::json!({
                    "slot_count": slot_count,
                    "signal": "cache_prelude",
                })),
            );
        }
        // Fallback: minifier mangled the `_c` alias (e.g. `var e=a(3)`); recover
        // the slot count from the always-surviving sentinel guard chain.
        if !emitted_prelude {
            let mut max_idx: i64 = -1;
            let mut span: Option<(usize, usize)> = None;
            for caps in SENTINEL_CHAIN_RE.captures_iter(source) {
                if let Some(idx) = caps.get(2).and_then(|m| m.as_str().parse::<i64>().ok()) {
                    if idx > max_idx {
                        max_idx = idx;
                        let whole = caps.get(0).unwrap();
                        span = Some((whole.start(), whole.end()));
                    }
                }
            }
            if let Some((start, end)) = span {
                let slot_count = (max_idx + 1) as u32;
                out.push(
                    AnalyzerMatch::new(
                        "react_compiler_fn",
                        format!("_c({slot_count})"),
                        PositionRange::from_byte_range(source, start, end),
                        file_type,
                        fp(),
                    )
                    .with_provenance(Provenance::Other)
                    .with_severity(Severity::Info)
                    .with_metadata(serde_json::json!({
                        "slot_count": slot_count,
                        "signal": "sentinel_chain",
                    })),
                );
            }
        }
    }

    // react_element_factory — `$$typeof` element symbol tags.
    for caps in ELEMENT_SYMBOL_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let symbol = caps.get(1).map_or("", |m| m.as_str()).to_string();
        out.push(
            AnalyzerMatch::new(
                "react_element_factory",
                symbol.clone(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                fp(),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info)
            .with_metadata(serde_json::json!({ "symbol": symbol })),
        );
    }

    // react_runtime_fingerprint — shared-internals / compiler-runtime export names.
    let has_client = source.contains("__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE");
    let has_legacy = source.contains("__SECRET_INTERNALS_DO_NOT_USE_OR_YOU_WILL_BE_FIRED");
    let has_compiler_runtime = source.contains("__COMPILER_RUNTIME");
    let version_era = if has_client {
        "react_19_plus"
    } else if has_legacy {
        "legacy_pre_19"
    } else {
        "unknown"
    };
    for caps in RUNTIME_FP_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let token = caps.get(1).map_or("", |m| m.as_str()).to_string();
        out.push(
            AnalyzerMatch::new(
                "react_runtime_fingerprint",
                token.clone(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                fp(),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info)
            .with_metadata(serde_json::json!({
                "token": token,
                "version_era": version_era,
                "has_compiler_runtime": has_compiler_runtime,
            })),
        );
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_compiler_fn_canonical_prelude() {
        // Shape from allow-global-mutation-in-effect-indirect-usecallback.expect.md.
        let src = concat!(
            r#"import { c as _c } from "react/compiler-runtime";"#,
            "\n",
            r#"function Component() {"#,
            "\n",
            r#"  const $ = _c(6);"#,
            "\n",
            r#"  let t0;"#,
            "\n",
            r#"  if ($[0] === Symbol.for("react.memo_cache_sentinel")) {"#,
            "\n",
            r#"    t0 = () => { setGlobal(); };"#,
            "\n",
            r#"    $[0] = t0;"#,
            "\n",
            r#"  } else { t0 = $[0]; }"#,
            "\n",
            r#"  return t0;"#,
            "\n",
            r#"}"#,
        );
        let m = analyze(src, Some("t.js"), FileType::Js);
        let fnmatch = m
            .iter()
            .find(|x| x.match_kind == "react_compiler_fn")
            .expect("react_compiler_fn");
        let meta = fnmatch.metadata.as_ref().unwrap();
        assert_eq!(meta["slot_count"], 6);
        assert_eq!(meta["signal"], "cache_prelude");
    }

    #[test]
    fn detects_compiler_fn_when_alias_mangled() {
        // Minified: the `_c` alias is mangled to `a`, so only the sentinel guard
        // chain survives — slot count is recovered from the max index (2 -> 3).
        let src = concat!(
            r#"function C(){var e=a(3);"#,
            r#"if(e[0]===Symbol.for("react.memo_cache_sentinel")){e[0]=x()}"#,
            r#"if(e[2]===Symbol.for("react.memo_cache_sentinel")){e[2]=y()}"#,
            r#"return e[2]}"#,
        );
        let m = analyze(src, Some("t.js"), FileType::Js);
        let fnmatch = m
            .iter()
            .find(|x| x.match_kind == "react_compiler_fn")
            .expect("react_compiler_fn (sentinel chain)");
        let meta = fnmatch.metadata.as_ref().unwrap();
        assert_eq!(meta["slot_count"], 3);
        assert_eq!(meta["signal"], "sentinel_chain");
    }

    #[test]
    fn detects_element_factory_symbols_excluding_sentinel() {
        // Minified element-symbol bank + a factory tagging `$$typeof`.
        let src = concat!(
            r#"var n=Symbol.for("react.transitional.element"),"#,
            r#"r=Symbol.for("react.forward_ref"),o=Symbol.for("react.memo"),"#,
            r#"i=Symbol.for("react.lazy"),u=Symbol.for("react.context");"#,
            r#"var s=Symbol.for("react.memo_cache_sentinel");"#,
            r#"function f(e){return{$$typeof:n,type:e,key:null,props:{}}}"#,
        );
        let m = analyze(src, Some("t.js"), FileType::Js);
        let syms: Vec<_> = m
            .iter()
            .filter(|x| x.match_kind == "react_element_factory")
            .map(|x| x.match_value.as_str())
            .collect();
        assert!(syms.contains(&"react.transitional.element"));
        assert!(syms.contains(&"react.forward_ref"));
        assert!(syms.contains(&"react.memo"));
        assert!(syms.contains(&"react.lazy"));
        assert!(syms.contains(&"react.context"));
        // The cache sentinel must NOT be misclassified as an element symbol.
        assert!(!syms.iter().any(|s| s.contains("memo_cache_sentinel")));
    }

    #[test]
    fn detects_runtime_fingerprint_era_and_compiler_runtime() {
        // React 19 client internals + compiler-runtime re-export (ReactClient.js).
        let src = concat!(
            r#"e.__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE=t;"#,
            r#"e.__COMPILER_RUNTIME=r;"#,
        );
        let m = analyze(src, Some("t.js"), FileType::Js);
        let fpm = m
            .iter()
            .find(|x| x.match_kind == "react_runtime_fingerprint")
            .expect("react_runtime_fingerprint");
        let meta = fpm.metadata.as_ref().unwrap();
        assert_eq!(meta["version_era"], "react_19_plus");
        assert_eq!(meta["has_compiler_runtime"], true);

        // Legacy (pre-19) internals name flips the era and clears the runtime flag.
        let legacy = r#"n.__SECRET_INTERNALS_DO_NOT_USE_OR_YOU_WILL_BE_FIRED=o;"#;
        let lm = analyze(legacy, None, FileType::Js);
        let lfp = lm
            .iter()
            .find(|x| x.match_kind == "react_runtime_fingerprint")
            .expect("legacy react_runtime_fingerprint");
        let lmeta = lfp.metadata.as_ref().unwrap();
        assert_eq!(lmeta["version_era"], "legacy_pre_19");
        assert_eq!(lmeta["has_compiler_runtime"], false);
    }
}
