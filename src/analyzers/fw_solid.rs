//! SolidJS recon analyzer. Fingerprints Solid / dom-expressions production bundles.
//!
//! Signatures that survive minification in production bundles; emits AnalyzerMatch.
//!
//! Literals were confirmed against the cloned upstream source at
//! `/home/cole/RustProjects/forks/frameworks/solid`:
//!   * `packages/solid/src/reactive/signal.ts`
//!       - L35 `export const $PROXY = Symbol("solid-proxy")`
//!       - L37 `export const $TRACK = Symbol("solid-track")`
//!       - L38 `export const $DEVCOMP = Symbol("solid-dev-component")`
//!       - L32 `export const IS_DEV = "_SOLID_DEV_"` (dev-build replacement flag)
//!       - L1236 `createMemo(..., undefined, { name: "children" })` (dev-only memo name)
//!   * `packages/solid/src/server/rendering.ts`
//!       - L96  `mapped += \`<!--!$-->\``                       (SSR sibling marker)
//!       - L363 `\`<!--!$e${id}-->...<!--!$/e${id}-->\``        (SSR error boundary markers)
//!       - L704 `\`<template id="pl-${id}"></template>...<!--pl-${id}-->\`` (Suspense placeholder)
//!
//! The `dom-expressions` runtime helpers (`_$template`/`_$insert`/`_$createComponent`/
//! `_$delegateEvents`/`_$HY`) and the `data-hk` hydration attribute are produced by the
//! `solid-js/web` runtime + `babel-plugin-jsx-dom-expressions` compile step. That package is
//! referenced from `packages/solid/package.json` (the `"./web"` export -> `web/dist`) but is
//! shipped pre-built, so it has no `.ts` source under `packages/solid/src`. These are the
//! canonical, well-documented compiler-emitted identifiers and are the most reliable Solid
//! bundle fingerprints; they are detected here on that basis.
//!
//! NOTE: `createSignal` / `createEffect` / `createMemo` identifiers do NOT survive
//! minification (they are renamed) and are deliberately NOT used as signatures.
use super::match_kind::{AnalyzerMatch, FileType, PositionRange, Provenance};
use super::regex_bank::Severity;
use regex::Regex;

/// Scan source for SolidJS / dom-expressions signatures.
#[must_use]
pub fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    let mut out = Vec::new();
    lazy_static::lazy_static! {
        // Solid internal symbols: the `"solid-*"` string literal survives ANY minifier
        // because it is the argument to `Symbol(...)`. Strongest single fingerprint.
        // Confirmed: signal.ts L35/L37/L38 ("solid-proxy"/"solid-track"/"solid-dev-component").
        static ref SOLID_SYMBOL_RE: Regex =
            Regex::new(r#"Symbol\(\s*["'`](solid-[a-z][a-z0-9-]*)["'`]\s*\)"#).unwrap();

        // dom-expressions runtime helpers emitted by babel-plugin-jsx-dom-expressions.
        // Canonical aliases from `solid-js/web` compiled output.
        static ref SOLID_DOM_HELPER_RE: Regex = Regex::new(
            r#"_\$(template|insert|createComponent|delegateEvents|effect|memo|spread|mergeProps|setAttribute|addEventListener|className|style|use|HY)\b"#
        ).unwrap();

        // Hoisted DOM template string: `_$template(`<div>...`)`. Capture the raw HTML.
        // Canonical dom-expressions output (always a backtick template literal).
        static ref SOLID_TEMPLATE_RE: Regex =
            Regex::new(r#"_\$template\(\s*`([^`]*)`"#).unwrap();

        // SSR / hydration boundary markers. Confirmed: rendering.ts L96/L363/L704.
        // Also matches the canonical `data-hk` hydration attribute from solid-js/web.
        static ref SOLID_HYDRATION_RE: Regex = Regex::new(
            r#"<!--!\$/?e?[0-9a-zA-Z]*-->|<!--pl-[0-9a-zA-Z]+-->|data-hk="[0-9a-z-]+""#
        ).unwrap();

        // Dev-only markers: the `_SOLID_DEV_` build flag and the named `{name:"children"}`
        // memo (only present when IS_DEV). Confirmed: signal.ts L32 and L1236.
        static ref SOLID_DEV_NAME_RE: Regex =
            Regex::new(r#"_SOLID_DEV_|\{\s*name:\s*"children"\s*\}"#).unwrap();
    }

    // solid_symbol -- captures the internal symbol name (e.g. "solid-proxy").
    for caps in SOLID_SYMBOL_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let val = caps.get(1).map_or("", |m| m.as_str()).to_string();
        out.push(
            AnalyzerMatch::new(
                "solid_symbol",
                val.clone(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                file_path.map(str::to_string),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Low)
            .with_metadata(serde_json::json!({ "symbol": val })),
        );
    }

    // solid_dom_helper -- dom-expressions runtime helper identifiers.
    for caps in SOLID_DOM_HELPER_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let helper = caps.get(1).map_or("", |m| m.as_str()).to_string();
        out.push(
            AnalyzerMatch::new(
                "solid_dom_helper",
                whole.as_str().to_string(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                file_path.map(str::to_string),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Low)
            .with_metadata(serde_json::json!({ "helper": helper })),
        );
    }

    // solid_template -- hoisted HTML template string (capture the html).
    for caps in SOLID_TEMPLATE_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let html = caps.get(1).map_or("", |m| m.as_str()).to_string();
        out.push(
            AnalyzerMatch::new(
                "solid_template",
                html.clone(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                file_path.map(str::to_string),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info)
            .with_metadata(serde_json::json!({ "html": html })),
        );
    }

    // solid_hydration -- SSR / hydration boundary markers.
    for whole in SOLID_HYDRATION_RE.find_iter(source) {
        out.push(
            AnalyzerMatch::new(
                "solid_hydration",
                whole.as_str().to_string(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                file_path.map(str::to_string),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info),
        );
    }

    // solid_dev_name -- dev-build-only markers (low signal). Gated behind the
    // `_SOLID_DEV_` flag: the `{name:"children"}` memo alternative is an ordinary
    // object literal that occurs in non-Solid bundles, so it is only a Solid
    // signature when the dev build is actually present.
    if source.contains("_SOLID_DEV_") {
        for whole in SOLID_DEV_NAME_RE.find_iter(source) {
            out.push(
                AnalyzerMatch::new(
                    "solid_dev_name",
                    whole.as_str().to_string(),
                    PositionRange::from_byte_range(source, whole.start(), whole.end()),
                    file_type,
                    file_path.map(str::to_string),
                )
                .with_provenance(Provenance::Other)
                .with_severity(Severity::Info)
                .with_metadata(serde_json::json!({ "dev_build": true })),
            );
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_core_signature() {
        // Production-shape Solid bundle: minified vars, hoisted template, dom helpers.
        let src = r#"var e=Symbol("solid-proxy"),t=Symbol("solid-track");const _tmpl$=/*#__PURE__*/_$template(`<div class=foo>x</div>`);_$insert(o,_$createComponent(C,{}));_$delegateEvents(["click"]);"#;
        let m = analyze(src, Some("t.js"), FileType::Js);
        assert!(
            m.iter()
                .any(|x| x.match_kind == "solid_symbol" && x.match_value == "solid-proxy")
        );
        assert!(m.iter().any(|x| x.match_kind == "solid_dom_helper"));
        assert!(
            m.iter()
                .any(|x| x.match_kind == "solid_template" && x.match_value == "<div class=foo>x</div>")
        );
    }

    #[test]
    fn detects_hydration_markers() {
        // Shapes emitted by server/rendering.ts SSR output.
        let src = r#"n+=`<!--!$-->`;r=`<!--!$e0-->y<!--!$/e0-->`;p+=`<template id="pl-0"></template><!--pl-0-->`;"#;
        let m = analyze(src, None, FileType::Js);
        let hits: Vec<_> = m.iter().filter(|x| x.match_kind == "solid_hydration").collect();
        assert!(
            hits.len() >= 3,
            "expected multiple hydration markers, got {}",
            hits.len()
        );
        assert!(hits.iter().any(|x| x.match_value == "<!--!$-->"));
    }

    #[test]
    fn detects_dev_name() {
        let src = r#"var IS_DEV="_SOLID_DEV_";const m=createMemo(r,void 0,{name:"children"});"#;
        let m = analyze(src, None, FileType::Js);
        assert!(
            m.iter()
                .any(|x| x.match_kind == "solid_dev_name" && x.match_value == "_SOLID_DEV_")
        );
        assert!(
            m.iter()
                .any(|x| x.match_kind == "solid_dev_name" && x.match_value.contains("children"))
        );
    }

    #[test]
    fn no_false_positive_on_plain_js() {
        let src = r#"const Symbol="x";function template(){return 1} let s="<div></div>";"#;
        let m = analyze(src, None, FileType::Js);
        assert!(m.is_empty(), "unexpected matches: {m:?}");
    }

    #[test]
    fn detects_single_quoted_symbol() {
        // Some minifiers emit single quotes; the `solid-*` token is unchanged.
        let m = analyze(r#"var e=Symbol('solid-proxy');"#, Some("t.js"), FileType::Js);
        assert!(
            m.iter()
                .any(|x| x.match_kind == "solid_symbol" && x.match_value == "solid-proxy"),
            "single-quoted Symbol missed: {m:?}"
        );
    }

    #[test]
    fn dev_name_requires_solid_dev_flag() {
        // A bare `{name:"children"}` object in a non-Solid bundle must NOT be
        // reported as a Solid dev signature.
        let plain = analyze(r#"var o={name:"children"};"#, Some("t.js"), FileType::Js);
        assert!(
            !plain.iter().any(|x| x.match_kind == "solid_dev_name"),
            "false-positive on plain object literal: {plain:?}"
        );
        // …but with the `_SOLID_DEV_` flag present it IS a Solid dev signature.
        let dev = analyze(
            r#"var _SOLID_DEV_=1;var o={name:"children"};"#,
            Some("t.js"),
            FileType::Js,
        );
        assert!(
            dev.iter().any(|x| x.match_kind == "solid_dev_name"),
            "should detect children memo in a dev build: {dev:?}"
        );
    }
}
