//! Vue 3 / Pinia framework recon analyzer. Fingerprints the Vue runtime + SFC
//! compiler output and the vue-router / Pinia ecosystem in production bundles.
//!
//! Signatures that survive minification in production bundles; emits `AnalyzerMatch`.
//
// Literal signatures confirmed against the cloned vuejs/core monorepo at
// /home/cole/RustProjects/forks/frameworks/vue:
//   * Symbol.for('v-fgt'|'v-txt'|'v-cmt'|'v-stc')
//       -> packages/runtime-core/src/vnode.ts:64-72
//   * __scopeId (rendered as the `data-v-<hash>` attribute prefix)
//       -> packages/runtime-core/src/component.ts:202,
//          packages/runtime-core/src/componentRenderContext.ts:27,
//          packages/runtime-dom/src/apiCustomElement.ts:491  (`key.startsWith('data-v-')`)
//   * __file / __name / __vccOpts / __asyncLoader component markers
//       -> packages/runtime-core/src/component.ts:218,222,246,
//          packages/runtime-core/src/apiAsyncComponent.ts:122
//   * resolveComponent("Name") + createStaticVNode("<html>")
//       -> packages/runtime-core/src/helpers/resolveAssets.ts:22,
//          packages/runtime-core/src/vnode.ts:763,
//          packages/compiler-core/src/runtimeHelpers.ts:99-100
//   * createRouter(...) (vue-router)
//       -> packages/runtime-core/src/componentPublicInstance.ts:67-77
//   * defineStore("id", {...}) (Pinia) — external package; the canonical store
//      factory whose call shape survives minification.

use super::match_kind::{AnalyzerMatch, FileType, PositionRange, Provenance};
use super::regex_bank::Severity;
use regex::Regex;
use std::sync::LazyLock;

static VNODE_SYM_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"Symbol\.for\(\s*["'](v-fgt|v-txt|v-cmt|v-stc)["']\s*\)"#).unwrap());
static SCOPE_ID_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"__scopeId\s*[:=]\s*["']data-v-([0-9a-fA-F]{4,16})["']"#).unwrap());
static MARKER_UNIQUE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(__vccOpts|__asyncLoader)\b").unwrap());
static MARKER_PROP_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(__file|__name)\s*:").unwrap());
static RESOLVE_COMP_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"resolveComponent\(\s*["']([A-Za-z_][\w-]*)["']"#).unwrap());
static STATIC_VNODE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"createStaticVNode\(\s*"((?:[^"\\]|\\.)*)""#).unwrap());
static ROUTE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"path\s*:\s*["']([^"']+)["']\s*,\s*(?:name\s*:\s*["'][^"']*["']\s*,\s*)?component\s*:\s*\(\s*\)\s*=>\s*import\(\s*["']([^"']+)["']"#,
    )
    .unwrap()
});
static CREATE_ROUTER_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bcreateRouter\s*\(").unwrap());
static PINIA_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\bdefineStore\(\s*["']([\w-]+)["']"#).unwrap());

/// Truncate a captured blob to at most `n` chars (char-safe, never splits UTF-8).
fn clip(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        let mut t: String = s.chars().take(n).collect();
        t.push('\u{2026}');
        t
    } else {
        s.to_string()
    }
}

fn emit_runtime_and_component_signatures(
    out: &mut Vec<AnalyzerMatch>,
    source: &str,
    file_path: Option<&str>,
    file_type: FileType,
) {
    for caps in VNODE_SYM_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let val = caps.get(1).map_or("", |m| m.as_str()).to_string();
        out.push(
            AnalyzerMatch::new(
                "vue_vnode_symbol",
                val,
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                file_path.map(str::to_string),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info),
        );
    }

    for caps in SCOPE_ID_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let hash = caps.get(1).map_or("", |m| m.as_str()).to_string();
        out.push(
            AnalyzerMatch::new(
                "vue_scope_id",
                format!("data-v-{hash}"),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                file_path.map(str::to_string),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info)
            .with_metadata(serde_json::json!({ "scope_hash": hash })),
        );
    }

    for re in [&*MARKER_UNIQUE_RE, &*MARKER_PROP_RE] {
        for caps in re.captures_iter(source) {
            let whole = caps.get(0).unwrap();
            let marker = caps.get(1).map_or("", |m| m.as_str()).to_string();
            out.push(
                AnalyzerMatch::new(
                    "vue_component_marker",
                    marker,
                    PositionRange::from_byte_range(source, whole.start(), whole.end()),
                    file_type,
                    file_path.map(str::to_string),
                )
                .with_provenance(Provenance::Other)
                .with_severity(Severity::Info),
            );
        }
    }
}

/// Scan source for Vue 3 / vue-router / Pinia signatures.
///
/// # Panics
///
/// Panics only if a compile-time regex pattern is invalid.
#[must_use]
pub fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    let mut out = Vec::new();
    emit_runtime_and_component_signatures(&mut out, source, file_path, file_type);

    // vue_component_ref — resolveComponent("Name").
    for caps in RESOLVE_COMP_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let name = caps.get(1).map_or("", |m| m.as_str()).to_string();
        out.push(
            AnalyzerMatch::new(
                "vue_component_ref",
                name.clone(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                file_path.map(str::to_string),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info)
            .with_metadata(serde_json::json!({ "component": name })),
        );
    }

    // vue_static_vnode — inlined static HTML from the SFC compiler.
    for caps in STATIC_VNODE_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let html = caps.get(1).map_or("", |m| m.as_str());
        out.push(
            AnalyzerMatch::new(
                "vue_static_vnode",
                clip(html, 200),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                file_path.map(str::to_string),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info)
            .with_metadata(serde_json::json!({
                "html_len": html.len(),
                "html_preview": clip(html, 120),
            })),
        );
    }

    // vue_route — lazy route entries (path + dynamically-imported chunk).
    for caps in ROUTE_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let path = caps.get(1).map_or("", |m| m.as_str()).to_string();
        let chunk = caps.get(2).map_or("", |m| m.as_str()).to_string();
        out.push(
            AnalyzerMatch::new(
                "vue_route",
                path.clone(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                file_path.map(str::to_string),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Low)
            .with_metadata(serde_json::json!({ "path": path, "chunk": chunk })),
        );
    }
    // vue_route — bare createRouter() presence (covers eager-loaded route tables).
    for m in CREATE_ROUTER_RE.find_iter(source) {
        out.push(
            AnalyzerMatch::new(
                "vue_route",
                "createRouter",
                PositionRange::from_byte_range(source, m.start(), m.end()),
                file_type,
                file_path.map(str::to_string),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Low)
            .with_metadata(serde_json::json!({ "signal": "createRouter" })),
        );
    }

    // pinia_store — defineStore("id", ...).
    for caps in PINIA_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let id = caps.get(1).map_or("", |m| m.as_str()).to_string();
        out.push(
            AnalyzerMatch::new(
                "pinia_store",
                id.clone(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                file_path.map(str::to_string),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Low)
            .with_metadata(serde_json::json!({ "store_id": id })),
        );
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_core_signature() {
        // Minified-shape Vue 3 runtime + SFC output.
        let src = r#"const Fe=Symbol.for("v-fgt"),Te=Symbol.for("v-txt"),Ce=Symbol.for("v-cmt"),Se=Symbol.for("v-stc");const _sfc_main={__name:"App",__file:"src/App.vue",setup(e,t){const o=resolveComponent("RouterLink");return()=>createStaticVNode("<div class=\"hero\"><h1>Hi</h1></div>",2)}};_sfc_main.__scopeId="data-v-7ba5bd90";"#;
        let m = analyze(src, Some("t.js"), FileType::Js);
        assert!(m.iter().any(|x| x.match_kind == "vue_vnode_symbol"));
        assert!(
            m.iter()
                .any(|x| x.match_kind == "vue_scope_id" && x.match_value == "data-v-7ba5bd90")
        );
        assert!(m.iter().any(|x| x.match_kind == "vue_component_marker"));
        assert!(
            m.iter()
                .any(|x| x.match_kind == "vue_component_ref" && x.match_value == "RouterLink")
        );
        assert!(m.iter().any(|x| x.match_kind == "vue_static_vnode"));
        // All four vnode symbols captured.
        assert_eq!(m.iter().filter(|x| x.match_kind == "vue_vnode_symbol").count(), 4);
    }

    #[test]
    fn detects_router_and_pinia() {
        let src = r#"const r=createRouter({history:h(),routes:[{path:"/users",name:"users",component:()=>import("./Users.a1b2c3.js")},{path:"/about",component:()=>import("./About.js")}]});const useCart=defineStore("cart",{state:()=>({items:[]})});"#;
        let m = analyze(src, None, FileType::Js);
        let routes: Vec<_> = m.iter().filter(|x| x.match_kind == "vue_route").collect();
        assert!(routes.iter().any(|x| x.match_value == "/users"));
        assert!(routes.iter().any(|x| x.match_value == "/about"));
        assert!(routes.iter().any(|x| x.match_value == "createRouter"));
        assert!(
            m.iter()
                .any(|x| x.match_kind == "pinia_store" && x.match_value == "cart")
        );
    }

    #[test]
    fn ignores_esbuild_name_helper() {
        // esbuild's __name helper must NOT register as a Vue component marker.
        let src = r#"var __name=(target,value)=>__defProp(target,"name",{value:value,configurable:!0});__name(MyComp,"MyComp");var x={"__name":"json"};"#;
        let m = analyze(src, None, FileType::Js);
        assert!(!m.iter().any(|x| x.match_kind == "vue_component_marker"));
    }
}
