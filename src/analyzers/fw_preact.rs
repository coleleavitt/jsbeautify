//! Preact recon analyzer. Fingerprints Preact core/hooks/compat in bundles.
//!
//! Signatures that survive minification in production bundles; emits AnalyzerMatch.
//!
//! Literals confirmed against the cloned current source at
//! `/home/cole/RustProjects/forks/frameworks/preact`:
//!   - `src/create-element.js` (createVNode `constructor: undefined`, isValidElement
//!     `vnode.constructor === undefined`) -> `preact_vnode`
//!   - `mangle.json` (`$_diff`->`__b`, `$_render`->`__r`, `$_commit`->`__c`,
//!     `$_catchError`->`__e`, `$__hooks`->`__H`, `$_list`->`__`,
//!     `$_pendingEffects`->`__h`, `$_forwarded`->`__f`) -> the minified key names
//!   - `hooks/src/index.js` (`options._diff/_render/diffed/_commit` installs and the
//!     `currentComponent.__hooks = { _list: [], _pendingEffects: [] }` container)
//!     -> `preact_options_hook`, `preact_hooks`
//!   - `compat/src/forwardRef.js` (`'ForwardRef(' + ...`) and `compat/src/memo.js`
//!     (`'Memo(' + ...`) -> `preact_compat`
//!
//! Note: `@preact/signals` is NOT vendored in this clone (no `signals/` dir, no
//! `signal` dep in `package.json`), so no signals detector is included.
use super::match_kind::{AnalyzerMatch, FileType, PositionRange, Provenance};
use super::regex_bank::Severity;
use regex::Regex;

/// Scan source for Preact framework signatures.
#[must_use]
pub fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    let mut out = Vec::new();
    lazy_static::lazy_static! {
        // Minified renderer option-hook installs. `_diff`->`__b`, `_render`->`__r`,
        // `_commit`->`__c`, `_catchError`->`__e` (mangle.json); `diffed`/`unmount`
        // are public, non-mangled option keys (hooks/src/index.js). Requiring a
        // function/arrow RHS disambiguates `__c`/`__e` from the overloaded
        // `_component`/`_dom` data fields.
        static ref OPTIONS_HOOK_RE: Regex =
            Regex::new(r#"\.(diffed|unmount|__[brce])\s*=\s*(?:function|[^=;]{0,40}?=>)"#).unwrap();
        // Hook-state container created in getHookState:
        // `__hooks = { _list: [], _pendingEffects: [] }` -> `{__:[],__h:[]}`.
        static ref HOOKS_CONTAINER_RE: Regex =
            Regex::new(r#"\{\s*__\s*:\s*\[\s*\]\s*,\s*__h\s*:\s*\[\s*\]\s*\}"#).unwrap();
        // VNode marker: createVNode sets `constructor: undefined` and isValidElement
        // tests `vnode.constructor === undefined` (src/create-element.js).
        // Minified -> `constructor:void 0` / `.constructor===void 0`.
        static ref VNODE_RE: Regex =
            Regex::new(r#"constructor\s*(?::|===?)\s*(void 0|undefined)"#).unwrap();
        // preact/compat forwardRef/memo shims set a `displayName` of the form
        // `'ForwardRef(' + ...` / `'Memo(' + ...` (compat/src/forwardRef.js,
        // compat/src/memo.js). React strips displayName in prod, so these literals
        // are Preact-compat specific.
        static ref COMPAT_SHIM_RE: Regex =
            Regex::new(r#"["'](ForwardRef|Memo)\("#).unwrap();
    }

    for caps in OPTIONS_HOOK_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let key = caps.get(1).map_or("", |m| m.as_str()).to_string();
        out.push(
            AnalyzerMatch::new(
                "preact_options_hook",
                key.clone(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                file_path.map(str::to_string),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info)
            .with_metadata(serde_json::json!({ "hook_key": key })),
        );
    }

    for caps in HOOKS_CONTAINER_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        out.push(
            AnalyzerMatch::new(
                "preact_hooks",
                whole.as_str().to_string(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                file_path.map(str::to_string),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info),
        );
    }

    for caps in VNODE_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let val = caps.get(1).map_or("", |m| m.as_str()).to_string();
        out.push(
            AnalyzerMatch::new(
                "preact_vnode",
                val,
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                file_path.map(str::to_string),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info),
        );
    }

    for caps in COMPAT_SHIM_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let shim = caps.get(1).map_or("", |m| m.as_str()).to_string();
        out.push(
            AnalyzerMatch::new(
                "preact_compat",
                shim.clone(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                file_path.map(str::to_string),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info)
            .with_metadata(serde_json::json!({ "shim": shim })),
        );
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_core_signature() {
        // Minified-shape preact/hooks options install + getHookState container.
        let src = r#"var i,t,r,o=0,u=[],c=l;c.__b=function(n){c.__b&&c.__b(n)},c.__r=function(n){f=n.__c},c.diffed=function(n){},c.__c=function(n,t){};function p(e){return e.__H||(e.__H={__:[],__h:[]})}"#;
        let m = analyze(src, Some("t.js"), FileType::Js);
        assert!(m.iter().any(|x| x.match_kind == "preact_options_hook"));
        assert!(m.iter().any(|x| x.match_kind == "preact_hooks"));
        // diffed + __b + __r + __c installs all captured.
        assert!(m.iter().filter(|x| x.match_kind == "preact_options_hook").count() >= 4);
    }

    #[test]
    fn detects_vnode_and_compat() {
        // Minified-shape createVNode literal, isValidElement check, and compat shims.
        let src = r#"function S(n,t,e,r,o){var u={type:n,props:t,key:e,ref:r,__k:null,__:null,__b:0,__e:null,__c:null,constructor:void 0,__v:null==o?++D:o,__i:-1,__u:0};return u}var isValid=function(n){return null!=n&&n.constructor===void 0};function fr(n){function t(t){return n(t,t.ref||null)}return t.$$typeof=R,t.render=n,t.prototype.isReactComponent=!0,t.displayName="ForwardRef("+(n.displayName||n.name)+")",t}function mo(n){function t(t){return E(n,t)}return t.displayName="Memo("+(n.displayName||n.name)+")",t.__f=!0,t}"#;
        let m = analyze(src, Some("t.js"), FileType::Js);
        assert!(m.iter().any(|x| x.match_kind == "preact_vnode"));
        let shims: Vec<_> = m
            .iter()
            .filter(|x| x.match_kind == "preact_compat")
            .map(|x| x.match_value.as_str())
            .collect();
        assert!(shims.contains(&"ForwardRef"));
        assert!(shims.contains(&"Memo"));
    }

    #[test]
    fn no_false_positives_on_unrelated_code() {
        let src = r#"const obj={foo:1,bar:[],baz:function(){return 42}};function render(){return obj.foo}"#;
        let m = analyze(src, None, FileType::Js);
        assert!(m.is_empty());
    }
}
