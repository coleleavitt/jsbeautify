//! Bun bundler runtime detection & rename
//!
//! Bun (1.4+) wraps every single-file executable in a CJS-style IIFE:
//! ```js
//! (function(exports, require, module, __filename, __dirname) {
//!     // ... entire program here, including the helper preamble
//! })
//! ```
//!
//! Inside that IIFE, Bun emits a canonical helper preamble of mangled names
//! (extracted ground truth from the bundler binary):
//!
//! | Canonical          | Mangled in cli.2.1.132.js | Body shape                                                |
//! |--------------------|---------------------------|-----------------------------------------------------------|
//! | `__create`         | `Sd4`                     | `Object.create`                                           |
//! | `__getProtoOf`     | `Rd4` (destructured)      | `Object.getPrototypeOf`                                   |
//! | `__defProp`        | `fjH` (destructured)      | `Object.defineProperty`                                   |
//! | `__getOwnPropNames`| `UP8` (destructured)      | `Object.getOwnPropertyNames`                              |
//! | `__getOwnPropDesc` | `Cd4` (destructured)      | `Object.getOwnPropertyDescriptor`                         |
//! | `__hasOwnProp`     | `XG$`                     | `Object.prototype.hasOwnProperty`                         |
//! | `__accessProp`     | `JG$`                     | `function (key) { return this[key] }`                     |
//! | `__commonJS`       | `n`                       | `(H,$) => () => ($\|\|H(($={exports:{}}).exports,$),...)` |
//! | `__esm`            | `V`                       | `(H,$) => () => (H && ($=H(H=0)), $)`                     |
//! | `__toESM`          | `m`                       | `(mod,isNode,target) => …`                                |
//! | `__toCommonJS`     | `n6`                      | WeakMap-cached `__esModule` wrapper                       |
//! | `__export`         | `j$`                      | `for (q in $) defProp(H,q,{get:$[q],…})`                  |
//!
//! This pass:
//! 1. Detects the top-level Bun CJS wrapper IIFE.
//! 2. Inside its body, identifies each helper by AST shape.
//! 3. Renames all references to the canonical name (e.g. `V → __esm`).
//! 4. Records the map in `DeobfuscateState` for downstream passes.
//!
//! No statements are deleted; this is purely a renaming pass. Subsequent passes
//! (`bun_esm_inliner`, `bun_commonjs_inliner`) consume the canonical names.

use oxc_traverse::TraverseCtx;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

mod ast_utils;
mod collector;
mod descriptor;
mod lowering;
mod matchers;
mod reexport;
mod renamer;
mod string_search;
mod types;

pub use collector::BunRuntimeCollector;
pub use renamer::BunRuntimeRenamer;
pub use types::{BunHelper, BunHelperMap, BundleRuntime, Bundler};

/// Parses `source` and detects its runtime helpers and the bundler they
/// identify, as the deobfuscation pipeline does in Phase 0.9.
#[must_use]
pub fn detect_bundle_runtime(source: &str) -> BundleRuntime {
    with_bundle_runtime(source, |_, _, collector| collector.into_runtime())
}

/// Parses `source`, detects its runtime helpers, and hands the program, its
/// scoping, and the collector (whose helper symbols resolve in that scoping)
/// to `inspect`.
pub fn with_bundle_runtime<T: Send>(
    source: &str,
    inspect: impl FnOnce(&oxc_ast::ast::Program<'_>, &oxc_semantic::Scoping, BunRuntimeCollector) -> T + Send,
) -> T {
    crate::stack::run_on_sized_stack(source.len(), || {
        let allocator = oxc_allocator::Allocator::default();
        let mut program = oxc_parser::Parser::new(&allocator, source, oxc_span::SourceType::unambiguous())
            .parse()
            .program;
        let scoping = oxc_semantic::SemanticBuilder::new()
            .build(&program)
            .semantic
            .into_scoping();
        let mut ctx = oxc_traverse::ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        let mut collector = BunRuntimeCollector::new();
        oxc_traverse::traverse_mut_with_ctx(&mut collector, &mut program, &mut ctx);
        inspect(&program, &ctx.into_scoping(), collector)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_allocator::Allocator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_semantic::SemanticBuilder;
    use oxc_span::SourceType;
    use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

    fn collect(code: &str) -> BunHelperMap {
        let alloc = Allocator::default();
        let parsed = Parser::new(&alloc, code, SourceType::mjs()).parse();
        let mut prog = parsed.program;
        let mut collector = BunRuntimeCollector::new();
        let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &alloc);
        traverse_mut_with_ctx(&mut collector, &mut prog, &mut ctx);
        collector.into_map()
    }

    fn collect_and_rename(code: &str) -> (String, BunHelperMap) {
        let alloc = Allocator::default();
        let parsed = Parser::new(&alloc, code, SourceType::mjs()).parse();
        let mut prog = parsed.program;

        let mut collector = BunRuntimeCollector::new();
        let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &alloc);
        traverse_mut_with_ctx(&mut collector, &mut prog, &mut ctx);
        let plan = collector
            .helper_symbols()
            .iter()
            .map(|(symbol, helper)| (*symbol, helper.canonical().to_string()))
            .collect();
        let map = collector.into_map();

        let mut renamer = BunRuntimeRenamer::new(plan);
        let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &alloc);
        traverse_mut_with_ctx(&mut renamer, &mut prog, &mut ctx);

        let out = Codegen::new().build(&prog).code;
        (out, map)
    }

    fn bundler(code: &str) -> Option<Bundler> {
        detect_bundle_runtime(code).bundler
    }

    /// The helper names `code` maps, sorted, with their canonical names.
    fn detected(code: &str) -> Vec<(String, &'static str)> {
        let mut helpers: Vec<_> = collect(code)
            .into_iter()
            .map(|(name, helper)| (name, helper.canonical()))
            .collect();
        helpers.sort_unstable();
        helpers
    }

    fn expect(pairs: &[(&str, &'static str)]) -> Vec<(String, &'static str)> {
        let mut pairs: Vec<_> = pairs
            .iter()
            .map(|(name, canonical)| (name.to_string(), *canonical))
            .collect();
        pairs.sort_unstable();
        pairs
    }

    #[test]
    fn detects_every_helper_of_a_minified_esbuild_0_28_bundle() {
        // The runtime preamble of `esbuild 0.28.2 --bundle --format=cjs --minify --target=es2016`.
        let code = r#"var W=Object.create;var p=Object.defineProperty,tt=Object.defineProperties,rt=Object.getOwnPropertyDescriptor,et=Object.getOwnPropertyDescriptors,nt=Object.getOwnPropertyNames,h=Object.getOwnPropertySymbols,st=Object.getPrototypeOf,F=Object.prototype.hasOwnProperty,E=Object.prototype.propertyIsEnumerable;var $=t=>{throw TypeError(t)};var z=(t,r,e)=>r in t?p(t,r,{enumerable:!0,configurable:!0,writable:!0,value:e}):t[r]=e,k=(t,r)=>{for(var e in r||(r={}))F.call(r,e)&&z(t,e,r[e]);if(h)for(var e of h(r))E.call(r,e)&&z(t,e,r[e]);return t},G=(t,r)=>tt(t,et(r));var H=t=>r=>{var e=t[r];if(e)return e();throw new Error("Module not found in bundle: "+r)};var I=(t,r)=>{var e={};for(var n in t)F.call(t,n)&&r.indexOf(n)<0&&(e[n]=t[n]);if(t!=null&&h)for(var n of h(t))r.indexOf(n)<0&&E.call(t,n)&&(e[n]=t[n]);return e};var ot=(t,r,e)=>()=>{if(e)throw e[0];try{return t&&(r=t(t=0)),r}catch(n){throw e=[n],n}};var b=(t,r)=>()=>{try{return r||t((r={exports:{}}).exports,r),r.exports}catch(e){throw r=0,e}},J=(t,r)=>{for(var e in r)p(t,e,{get:r[e],enumerable:!0})},l=(t,r,e,n)=>{if(r&&typeof r=="object"||typeof r=="function")for(let u of nt(r))!F.call(t,u)&&u!==e&&p(t,u,{get:()=>r[u],enumerable:!(n=rt(r,u))||n.enumerable});return t},o=(t,r,e)=>(l(t,r,"default"),e&&l(e,r,"default")),K=(t,r,e)=>(e=t!=null?W(st(t)):{},l(r||!t||!t.__esModule?p(e,"default",{value:t,enumerable:!0}):e,t)),L=t=>l(p({},"__esModule",{value:!0}),t);var x=(t,r,e)=>z(t,typeof r!="symbol"?r+"":r,e),v=(t,r,e)=>r.has(t)||$("Cannot "+e),M=(t,r)=>Object(r)!==r?$('Cannot use the "in" operator on this value'):t.has(r),d=(t,r,e)=>(v(t,r,"read from private field"),e?e.call(t):r.get(t)),m=(t,r,e)=>r.has(t)?$("Cannot add the same private member more than once"):r instanceof WeakSet?r.add(t):r.set(t,e),N=(t,r,e,n)=>(v(t,r,"write to private field"),n?n.call(t,e):r.set(t,e),e),O=(t,r,e)=>(v(t,r,"access private method"),e);var Q=(t,r,e)=>new Promise((n,u)=>{var P=i=>{try{w(e.next(i))}catch(j){u(j)}},S=i=>{try{w(e.throw(i))}catch(j){u(j)}},w=i=>i.done?n(i.value):Promise.resolve(i.value).then(P,S);w((e=e.apply(t,r)).next())});"#;
        assert_eq!(
            detected(code),
            expect(&[
                ("W", "__create"),
                ("p", "__defProp"),
                ("tt", "__defProps"),
                ("rt", "__getOwnPropDesc"),
                ("et", "__getOwnPropDescs"),
                ("nt", "__getOwnPropNames"),
                ("h", "__getOwnPropSymbols"),
                ("st", "__getProtoOf"),
                ("F", "__hasOwnProp"),
                ("E", "__propIsEnum"),
                ("$", "__typeError"),
                ("z", "__defNormalProp"),
                ("k", "__spreadValues"),
                ("G", "__spreadProps"),
                ("H", "__glob"),
                ("I", "__objRest"),
                ("ot", "__esm"),
                ("b", "__commonJS"),
                ("J", "__export"),
                ("l", "__copyProps"),
                ("o", "__reExport"),
                ("K", "__toESM"),
                ("L", "__toCommonJS"),
                ("x", "__publicField"),
                ("v", "__accessCheck"),
                ("M", "__privateIn"),
                ("d", "__privateGet"),
                ("m", "__privateAdd"),
                ("N", "__privateSet"),
                ("O", "__privateMethod"),
                ("Q", "__async"),
            ])
        );
    }

    #[test]
    fn detects_the_helpers_of_a_minified_esbuild_0_14_bundle() {
        // Before esbuild 0.21 the private-member helpers throw instead of calling `__typeError`.
        let code = r#"var X=Object.create;var p=Object.defineProperty,Y=Object.defineProperties,Z=Object.getOwnPropertyDescriptor,_=Object.getOwnPropertyDescriptors,P=Object.getOwnPropertyNames,h=Object.getOwnPropertySymbols,S=Object.getPrototypeOf,z=Object.prototype.hasOwnProperty,C=Object.prototype.propertyIsEnumerable;var j=(t,r,e)=>r in t?p(t,r,{enumerable:!0,configurable:!0,writable:!0,value:e}):t[r]=e,F=(t,r)=>{for(var e in r||(r={}))z.call(r,e)&&j(t,e,r[e]);if(h)for(var e of h(r))C.call(r,e)&&j(t,e,r[e]);return t},D=(t,r)=>Y(t,_(r));var E=(t,r)=>{var e={};for(var n in t)z.call(t,n)&&r.indexOf(n)<0&&(e[n]=t[n]);if(t!=null&&h)for(var n of h(t))r.indexOf(n)<0&&C.call(t,n)&&(e[n]=t[n]);return e};var W=(t,r)=>()=>(t&&(r=t(t=0)),r);var G=(t,r)=>()=>(r||t((r={exports:{}}).exports,r),r.exports),H=(t,r)=>{for(var e in r)p(t,e,{get:r[e],enumerable:!0})},l=(t,r,e,n)=>{if(r&&typeof r=="object"||typeof r=="function")for(let u of P(r))!z.call(t,u)&&u!==e&&p(t,u,{get:()=>r[u],enumerable:!(n=Z(r,u))||n.enumerable});return t},o=(t,r,e)=>(l(t,r,"default"),e&&l(e,r,"default")),I=(t,r,e)=>(e=t!=null?X(S(t)):{},l(r||!t||!t.__esModule?p(e,"default",{value:t,enumerable:!0}):e,t)),J=t=>l(p({},"__esModule",{value:!0}),t);var x=(t,r,e)=>(j(t,typeof r!="symbol"?r+"":r,e),e),$=(t,r,e)=>{if(!r.has(t))throw TypeError("Cannot "+e)},K=(t,r)=>{if(Object(r)!==r)throw TypeError('Cannot use the "in" operator on this value');return t.has(r)},b=(t,r,e)=>($(t,r,"read from private field"),e?e.call(t):r.get(t)),m=(t,r,e)=>{if(r.has(t))throw TypeError("Cannot add the same private member more than once");r instanceof WeakSet?r.add(t):r.set(t,e)},L=(t,r,e,n)=>($(t,r,"write to private field"),n?n.call(t,e):r.set(t,e),e);var M=(t,r,e)=>($(t,r,"access private method"),e);var N=(t,r,e)=>new Promise((n,u)=>{var U=i=>{try{T(e.next(i))}catch(w){u(w)}},V=i=>{try{T(e.throw(i))}catch(w){u(w)}},T=i=>i.done?n(i.value):Promise.resolve(i.value).then(U,V);T((e=e.apply(t,r)).next())});"#;
        assert_eq!(
            detected(code),
            expect(&[
                ("X", "__create"),
                ("p", "__defProp"),
                ("Y", "__defProps"),
                ("Z", "__getOwnPropDesc"),
                ("_", "__getOwnPropDescs"),
                ("P", "__getOwnPropNames"),
                ("h", "__getOwnPropSymbols"),
                ("S", "__getProtoOf"),
                ("z", "__hasOwnProp"),
                ("C", "__propIsEnum"),
                ("j", "__defNormalProp"),
                ("F", "__spreadValues"),
                ("D", "__spreadProps"),
                ("E", "__objRest"),
                ("W", "__esm"),
                ("G", "__commonJS"),
                ("H", "__export"),
                ("l", "__copyProps"),
                ("o", "__reExport"),
                ("I", "__toESM"),
                ("J", "__toCommonJS"),
                ("x", "__publicField"),
                ("$", "__accessCheck"),
                ("K", "__privateIn"),
                ("b", "__privateGet"),
                ("m", "__privateAdd"),
                ("L", "__privateSet"),
                ("M", "__privateMethod"),
                ("N", "__async"),
            ])
        );
    }

    #[test]
    fn detects_the_unminified_lazy_wrappers() {
        // esbuild 0.28 (`try` + rethrow) and 0.14 forms without `--minify`.
        let code = r"var __getOwnPropNames = Object.getOwnPropertyNames;
            var E = (fn, res, err) => function __init() {
                if (err) throw err[0];
                try { return fn && (res = (0, fn[__getOwnPropNames(fn)[0]])(fn = 0)), res; } catch (e) { throw err = [e], e; }
            };
            var C = (cb, mod) => function __require() {
                try { return mod || (0, cb[__getOwnPropNames(cb)[0]])((mod = { exports: {} }).exports, mod), mod.exports; } catch (e) { throw mod = 0, e; }
            };
            var E0 = (fn, res) => function __init() { return fn && (res = (0, fn[__getOwnPropNames(fn)[0]])(fn = 0)), res; };
            var C0 = (cb, mod) => function __require() { return mod || (0, cb[__getOwnPropNames(cb)[0]])((mod = { exports: {} }).exports, mod), mod.exports; };";
        let map = collect(code);
        for (name, helper) in [
            ("E", BunHelper::Esm),
            ("C", BunHelper::CommonJs),
            ("E0", BunHelper::Esm),
            ("C0", BunHelper::CommonJs),
        ] {
            assert_eq!(map.get(name), Some(&helper), "{name}");
        }
    }

    #[test]
    fn rejects_lazy_wrapper_lookalikes() {
        for code in [
            // No rethrow guard for the third parameter.
            "var E = (t, r, e) => () => { try { return t && (r = t(t = 0)), r } catch (n) { throw n } };",
            // Returns something other than the cached result.
            "var E = (t, r) => () => (t && (r = t(t = 0)), t);",
            "var C = (t, r) => () => (r || t((r = { exports: {} }).exports, r), t.exports);",
            "var E = (t, r) => () => { log(); return t && (r = t(t = 0)), r };",
        ] {
            assert!(collect(code).is_empty(), "{code}");
        }
    }

    #[test]
    fn detects_the_helpers_below_es6() {
        // `--supported:for-of=false`: indexed loops and `.bind` getters.
        let code = r#"var f = Object.defineProperty, nt = Object.getOwnPropertyNames, rt = Object.getOwnPropertyDescriptor,
            d = Object.getOwnPropertySymbols, k = Object.prototype.hasOwnProperty, H = Object.prototype.propertyIsEnumerable;
            var $ = (t, r, e) => r in t ? f(t, r, { enumerable: !0, configurable: !0, writable: !0, value: e }) : t[r] = e,
            B=(t,r)=>{for(var e in r||(r={}))k.call(r,e)&&$(t,e,r[e]);if(d)for(var n=d(r),u=0,i=n.length,e;u<i;u++)e=n[u],H.call(r,e)&&$(t,e,r[e]);return t};
            var y=(t,r,e,n)=>{if(r&&typeof r=="object"||typeof r=="function")for(var u=nt(r),i=0,m=u.length,a;i<m;i++)a=u[i],!k.call(t,a)&&a!==e&&f(t,a,{get:(c=>r[c]).bind(null,a),enumerable:!(n=rt(r,a))||n.enumerable});return t};"#;
        let map = collect(code);
        assert_eq!(map.get("B"), Some(&BunHelper::SpreadValues));
        assert_eq!(map.get("y"), Some(&BunHelper::CopyProps));
    }

    #[test]
    fn detects_the_keep_names_and_require_shims() {
        let code = r#"var i = Object.defineProperty;
            var s = (r, t) => i(r, "name", { value: t, configurable: !0 });
            var E=(t=>typeof require!="undefined"?require:typeof Proxy!="undefined"?new Proxy(t,{get:(r,e)=>(typeof require!="undefined"?require:r)[e]}):t)(function(t){if(typeof require!="undefined")return require.apply(this,arguments);throw new Error('Dynamic require of "'+t+'" is not supported')});"#;
        let map = collect(code);
        assert_eq!(map.get("s"), Some(&BunHelper::Name));
        assert_eq!(map.get("E"), Some(&BunHelper::Require));
    }

    #[test]
    fn descends_into_an_iife_bundle() {
        let code = "var lib = (() => { var a = Object.defineProperty; var b = (t, r) => () => (t && (r = t(t = 0)), r); return 1; })();
            (() => { var c = Object.create; })();";
        assert_eq!(
            detected(code),
            expect(&[("a", "__defProp"), ("b", "__esm"), ("c", "__create")])
        );
    }

    #[test]
    fn identifies_esbuild_from_its_export_descriptor() {
        let code = "var d = Object.defineProperty;
            var X = (e, t) => { for (var n in t) d(e, n, { get: t[n], enumerable: !0 }); };";
        assert_eq!(bundler(code), Some(Bundler::Esbuild));
    }

    #[test]
    fn identifies_bun_from_its_export_setter() {
        let code = "var d = Object.defineProperty;
            var X = (e, t) => { for (var n in t) d(e, n, { get: t[n], enumerable: !0, configurable: !0, set: (r) => t[n] = () => r }); };";
        assert_eq!(bundler(code), Some(Bundler::Bun));
    }

    #[test]
    fn identifies_bun_from_its_weak_map_cache() {
        let code = r"var G, R = (e, r, t) => {
                var c = G ??= new WeakMap, d = c.get(e);
                if (d) return d;
                t = {};
                return e && e.__esModule ? e : t;
            };";
        assert_eq!(bundler(code), Some(Bundler::Bun));
    }

    #[test]
    fn identifies_bun_from_its_pragma() {
        assert_eq!(bundler("// @bun @bytecode @bun-cjs\nvar a = 1;"), Some(Bundler::Bun));
        assert_eq!(bundler("var a = 1; // @bun"), None);
    }

    #[test]
    fn leaves_the_bundler_unknown_without_a_distinguishing_helper() {
        assert_eq!(bundler("var E = (e, t) => () => (e && (t = e(e = 0)), t);"), None);
        let both = "var d = Object.defineProperty;
            var X = (e, t) => { for (var n in t) d(e, n, { get: t[n], enumerable: !0 }); };
            var Y = (e, t) => { for (var n in t) d(e, n, { get: t[n], enumerable: !0, set: (r) => r }); };";
        assert_eq!(bundler(both), None);
    }

    #[test]
    fn rejects_for_in_lookalikes_as_export_helper() {
        let map = collect("var X = (e, t) => { for (var n in t) e[n] = t[n]; };");
        assert_eq!(map.get("X"), None);
        let map = collect("var X = (e, t) => { for (var n in t) d(t, n, { get: e[n] }); };");
        assert_eq!(map.get("X"), None);
    }

    #[test]
    fn detects_esm_helper() {
        let map = collect("var V = (H, $) => () => (H && ($ = H(H = 0)), $);");
        assert_eq!(map.get("V"), Some(&BunHelper::Esm));
    }

    #[test]
    fn detects_commonjs_helper() {
        let map = collect("var n = (H, $) => () => ($ || H(($ = { exports: {} }).exports, $), $.exports);");
        assert_eq!(map.get("n"), Some(&BunHelper::CommonJs));
    }

    #[test]
    fn detects_reexport_before_copyprops() {
        let map = collect(
            r#"
            var __defProp = Object.defineProperty;
            var __getOwnPropNames = Object.getOwnPropertyNames;
            var R = (H, $, q) => {
                var K = __getOwnPropNames($);
                for (let _ of K) if (_ !== "default") __defProp(H, _, { get: () => $[_], enumerable: true });
                return q;
            };
            "#,
        );
        assert_eq!(map.get("R"), Some(&BunHelper::ReExport));
    }

    #[test]
    fn does_not_detect_broad_reexport_lookalike() {
        let map = collect(
            r#"
            var R = (H, $, q) => {
                var K = list($);
                for (let _ of K) if (_ !== "default") H[_] = $[_];
                return q;
            };
            "#,
        );
        assert_eq!(map.get("R"), None);
    }

    #[test]
    fn detects_copyprops_with_get_own_property_names_loop() {
        let map = collect(
            r"
            var C = (H, $, q) => {
                var K = Object.getOwnPropertyNames($);
                for (let _ of K) H[_] = $[_];
            };
            ",
        );
        assert_eq!(map.get("C"), Some(&BunHelper::CopyProps));
    }

    #[test]
    fn detects_destructured_object_aliases() {
        let map =
            collect("var { defineProperty: fjH, getOwnPropertyNames: UP8, getOwnPropertyDescriptor: Cd4 } = Object;");
        assert_eq!(map.get("fjH"), Some(&BunHelper::DefProp));
        assert_eq!(map.get("UP8"), Some(&BunHelper::GetOwnPropNames));
        assert_eq!(map.get("Cd4"), Some(&BunHelper::GetOwnPropDesc));
    }

    #[test]
    fn detects_create_alias() {
        let map = collect("var Sd4 = Object.create;");
        assert_eq!(map.get("Sd4"), Some(&BunHelper::Create));
    }

    #[test]
    fn detects_hasownprop() {
        let map = collect("var XG$ = Object.prototype.hasOwnProperty;");
        assert_eq!(map.get("XG$"), Some(&BunHelper::HasOwnProp));
    }

    #[test]
    fn detects_accessprop_function() {
        let map = collect("function JG$(H) { return this[H]; }");
        assert_eq!(map.get("JG$"), Some(&BunHelper::AccessProp));
    }

    #[test]
    fn descends_into_cjs_wrapper_iife() {
        let code = r"
            (function(exports, require, module, __filename, __dirname) {
                var V = (H, $) => () => (H && ($ = H(H = 0)), $);
                var n = (H, $) => () => ($ || H(($ = { exports: {} }).exports, $), $.exports);
                var Sd4 = Object.create;
            })(exports, require, module, __filename, __dirname);
        ";
        let map = collect(code);
        assert_eq!(map.get("V"), Some(&BunHelper::Esm));
        assert_eq!(map.get("n"), Some(&BunHelper::CommonJs));
        assert_eq!(map.get("Sd4"), Some(&BunHelper::Create));
    }

    #[test]
    fn renames_references() {
        let code = r#"
            var V = (H, $) => () => (H && ($ = H(H = 0)), $);
            var modA = V(() => { console.log("a"); });
            var modB = V(() => { console.log("b"); });
        "#;
        let (out, _) = collect_and_rename(code);
        assert!(out.contains("__esm"), "Should rename V to __esm: {out}");
        assert!(!out.contains(" V("), "Should not retain V() calls: {out}");
        // The decl should also be renamed.
        assert!(out.contains("var __esm ="));
    }

    #[test]
    fn rename_leaves_shadowing_binding_with_same_name() {
        let (out, _) = collect_and_rename("var e = Object.create; function f(e) { return e + 1; } f(typeof e);");
        assert!(out.contains("var __create = Object.create"), "got: {out}");
        assert!(out.contains("function f(e)"), "got: {out}");
        assert!(out.contains("return e + 1"), "got: {out}");
        assert!(out.contains("typeof __create"), "got: {out}");
    }

    #[test]
    fn renames_destructured_aliases() {
        let code = r#"
            var { defineProperty: fjH } = Object;
            fjH(target, "x", { value: 1 });
        "#;
        let (out, _) = collect_and_rename(code);
        assert!(out.contains("__defProp"), "Should rename fjH to __defProp: {out}");
        assert!(!out.contains("fjH"), "Should not retain fjH: {out}");
    }
}
