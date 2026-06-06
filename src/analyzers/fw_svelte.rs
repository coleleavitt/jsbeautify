//! Svelte 5 recon analyzer. Detects Svelte runtime/compiler fingerprints that
//! survive minification in production bundles — hoisted template strings (which
//! leak routes/forms/classes), scoped-style hashes, store references, the public
//! version marker, and the per-component `$$css` style object.
//!
//! Signatures that survive minification in production bundles; emits AnalyzerMatch.
//!
//! Signatures confirmed against the cloned Svelte source (packages/svelte/src):
//!   - `from_html` / `from_svg` / `from_mathml` / `from_tree` hoisted templates:
//!     internal/client/dom/template.js (from_html L53, from_namespace L101,
//!     from_svg L154, from_mathml L163, from_tree L219); codegen emits
//!     `$.from_${namespace}(\`...\`, flags)` /  `$.from_tree([...])` in
//!     compiler/phases/3-transform/client/transform-template/index.js (L29-31).
//!   - `svelte-<hash>` scoped class: compiler/validate-options.js L78
//!     (`svelte-${hash(...)}`); hash is `(n>>>0).toString(36)` base36, utils.js L13.
//!     Applied to class attrs in 3-transform/client/visitors/RegularElement.js L245-249.
//!   - `store_get(ref,'$name',...)`: internal/client/reactivity/store.js L36 and
//!     internal/server/index.js L243 (`store_name` is the `$`-prefixed auto-sub name).
//!   - `window.__svelte.v` version Set: internal/disclose-version.js L5
//!     (`((window.__svelte ??= {}).v ??= new Set()).add(PUBLIC_VERSION)`).
//!   - `$$css = { hash, code }` style object: compiler/phases/3-transform/client/
//!     transform-client.js L413-419 (`b.const('$$css', { hash, code })` +
//!     `$.append_styles($$anchor, $$css)`).

use super::match_kind::{AnalyzerMatch, FileType, PositionRange, Provenance};
use super::regex_bank::Severity;
use lazy_static::lazy_static;
use regex::Regex;

lazy_static! {
    /// Hoisted template call: `$.from_html(`<...>`, flags)` (also `from_svg` /
    /// `from_mathml`). The `$.` namespace prefix is optional/renameable, so we
    /// only anchor on the stable export name + an immediately-following string
    /// literal. Group 1 = variant; groups 2/3/4 = backtick/single/double html.
    /// template.js L53/L154/L163; codegen transform-template/index.js L29.
    static ref TEMPLATE_RE: Regex = Regex::new(
        r#"(?:[\w$]+\.)?from_(html|svg|mathml)\(\s*(?:`((?:[^`\\]|\\.)*)`|'((?:[^'\\]|\\.)*)'|"((?:[^"\\]|\\.)*)")"#,
    )
    .unwrap();

    /// Tree-mode template: `$.from_tree([...])` (`fragments: 'tree'` builds).
    /// Capture group 1 is the leading structure segment (reveals tag names).
    /// template.js L219; codegen transform-template/index.js L29.
    static ref TREE_RE: Regex =
        Regex::new(r#"(?:[\w$]+\.)?from_tree\(\s*\[([^\]]{0,200})"#).unwrap();

    /// `href`/`action`/`src`/`formaction` attribute values inside a captured
    /// template — surfaced as endpoint metadata.
    static ref ATTR_ENDPOINT_RE: Regex =
        Regex::new(r#"(?:href|action|formaction|src)\s*=\s*["']([^"']+)["']"#).unwrap();

    /// Scoped-style class `svelte-<hash>`, hash = base36 of a u32 (4-7 chars).
    /// `\b` keeps it a standalone token so the word-based internal classes
    /// (`svelte-css-wrapper`, `svelte-element`, ...) don't bleed in; a digit
    /// requirement (enforced in code) filters the remaining all-letter words.
    /// validate-options.js L78, utils.js L13.
    static ref SCOPE_CLASS_RE: Regex = Regex::new(r#"svelte-([0-9a-z]{4,8})\b"#).unwrap();

    /// Auto-subscription read: `store_get(ref,'$name',stores)`. Capture group 1
    /// is the `$`-prefixed store name. store.js L36 / server/index.js L243.
    static ref STORE_REF_RE: Regex =
        Regex::new(r#"store_get\([^,]*,\s*["'](\$[A-Za-z_$][A-Za-z0-9_$]*)["']"#).unwrap();

    /// Public version marker `((window.__svelte ??= {}).v ??= new Set()).add("x.y.z")`.
    /// Requires `.v` + `.add(<semver>)` so the sibling `.uid` / `.h` accesses don't
    /// match. Capture group 1 = inlined PUBLIC_VERSION. disclose-version.js L5.
    static ref VERSION_RE: Regex = Regex::new(
        r#"window\.__svelte[^;]{0,160}?\.v\b[^;]{0,160}?\.add\(\s*["']([0-9]+\.[0-9]+\.[0-9]+[^"']*)["']"#,
    )
    .unwrap();

    /// Per-component style object `$$css = { hash:'svelte-...', code:... }`,
    /// fed to `$.append_styles`. Capture group 1 = the scope hash; the trailing
    /// `,code:` anchor keeps this svelte-specific. transform-client.js L413-419.
    static ref CSS_RE: Regex =
        Regex::new(r#"\$\$css\s*=\s*\{\s*hash:\s*["'](svelte-[0-9a-z]+)["']\s*,\s*code:"#).unwrap();
}

/// Pull `href`/`action`/`src` endpoint values out of a captured template string.
fn template_endpoints(html: &str) -> Vec<String> {
    ATTR_ENDPOINT_RE
        .captures_iter(html)
        .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
        .collect()
}

/// Scan source for Svelte signatures.
#[must_use]
pub fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    let mut out = Vec::new();
    let fp = || file_path.map(str::to_string);

    // svelte_template — hoisted from_html/from_svg/from_mathml strings (the html
    // reveals routes, form actions, and scoped class names).
    for caps in TEMPLATE_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let variant = caps.get(1).map_or("html", |m| m.as_str());
        let html = caps
            .get(2)
            .or_else(|| caps.get(3))
            .or_else(|| caps.get(4))
            .map_or("", |m| m.as_str());
        // Require a tag to keep this high-precision (skip from_html-named user fns).
        if !html.contains('<') {
            continue;
        }
        let endpoints = template_endpoints(html);
        out.push(
            AnalyzerMatch::new(
                "svelte_template",
                html.to_string(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                fp(),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info)
            .with_metadata(serde_json::json!({ "variant": variant, "endpoints": endpoints })),
        );
    }

    // svelte_template (tree variant) — `from_tree([...])` structure builds.
    for caps in TREE_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let structure = caps.get(1).map_or("", |m| m.as_str()).to_string();
        out.push(
            AnalyzerMatch::new(
                "svelte_template",
                structure.clone(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                fp(),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info)
            .with_metadata(serde_json::json!({ "variant": "tree", "structure": structure })),
        );
    }

    // svelte_scope_class — `svelte-<hash>` scoped style class join keys.
    for caps in SCOPE_CLASS_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let hash = caps.get(1).map_or("", |m| m.as_str());
        // The base36 css hash effectively always carries a digit; requiring one
        // discards the word-based internal classes (`svelte-element`, etc.).
        if !hash.bytes().any(|b| b.is_ascii_digit()) {
            continue;
        }
        out.push(
            AnalyzerMatch::new(
                "svelte_scope_class",
                whole.as_str().to_string(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                fp(),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info),
        );
    }

    // svelte_store_ref — `store_get(ref,'$name',...)` auto-subscription reads.
    for caps in STORE_REF_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let name = caps.get(1).map_or("", |m| m.as_str()).to_string();
        out.push(
            AnalyzerMatch::new(
                "svelte_store_ref",
                name.clone(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                fp(),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info)
            .with_metadata(serde_json::json!({ "store": name })),
        );
    }

    // svelte_version — `window.__svelte.v` Set with the inlined PUBLIC_VERSION.
    for caps in VERSION_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let version = caps.get(1).map_or("", |m| m.as_str()).to_string();
        out.push(
            AnalyzerMatch::new(
                "svelte_version",
                version.clone(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                fp(),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info)
            .with_metadata(serde_json::json!({ "version": version })),
        );
    }

    // svelte_css — `$$css = { hash, code }` per-component style object.
    for caps in CSS_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let scope = caps.get(1).map_or("", |m| m.as_str()).to_string();
        out.push(
            AnalyzerMatch::new(
                "svelte_css",
                scope.clone(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                fp(),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info)
            .with_metadata(serde_json::json!({ "scope": scope })),
        );
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_template_routes_and_scope_class() {
        // Minified shape from transform-template/index.js codegen (`$.from_html(`...`)`)
        // with the scoped class join from RegularElement.js L245-249.
        let src = r#"var root=$.from_html(`<form action="/api/login" method="post" class="card svelte-1x9z7q"><a href="/dashboard/settings">go</a></form>`);"#;
        let m = analyze(src, Some("t.js"), FileType::Js);

        let t = m.iter().find(|x| x.match_kind == "svelte_template").unwrap();
        assert!(t.match_value.contains("/api/login"));
        let meta = t.metadata.as_ref().unwrap();
        let endpoints = meta["endpoints"].as_array().unwrap();
        assert!(endpoints.iter().any(|e| e.as_str() == Some("/api/login")));
        assert!(endpoints.iter().any(|e| e.as_str() == Some("/dashboard/settings")));

        assert!(
            m.iter()
                .any(|x| x.match_kind == "svelte_scope_class" && x.match_value == "svelte-1x9z7q")
        );
    }

    #[test]
    fn detects_store_version_and_css() {
        // store.js L36, disclose-version.js L5, transform-client.js L413-419 shapes.
        let src = concat!(
            r#"const count=$.store_get(c,'$count',$$stores);"#,
            "\n",
            r#"((window.__svelte??={}).v??=new Set).add("5.39.5");"#,
            "\n",
            r#"const $$css={hash:'svelte-1x9z7q',code:'.card.svelte-1x9z7q{color:red}'};$.append_styles($$anchor,$$css);"#,
        );
        let m = analyze(src, None, FileType::Js);

        assert!(
            m.iter()
                .any(|x| x.match_kind == "svelte_store_ref" && x.match_value == "$count")
        );
        assert!(
            m.iter()
                .any(|x| x.match_kind == "svelte_version" && x.match_value == "5.39.5")
        );
        assert!(
            m.iter()
                .any(|x| x.match_kind == "svelte_css" && x.match_value == "svelte-1x9z7q")
        );
    }

    #[test]
    fn detects_svg_and_tree_templates() {
        // from_svg (template.js L154) + from_tree (L219) variants.
        let src = concat!(
            r#"var icon=$.from_svg(`<svg viewBox="0 0 24 24"><path d="M0 0z"/></svg>`);"#,
            r#"var t=$.from_tree([['div',{},'hi']]);"#,
        );
        let m = analyze(src, None, FileType::Js);

        let variants: Vec<String> = m
            .iter()
            .filter(|x| x.match_kind == "svelte_template")
            .filter_map(|x| x.metadata.as_ref())
            .filter_map(|md| md.get("variant").and_then(|v| v.as_str()).map(String::from))
            .collect();
        assert!(variants.len() >= 2);
        assert!(variants.iter().any(|v| v == "svg"));
        assert!(variants.iter().any(|v| v == "tree"));
    }
}
