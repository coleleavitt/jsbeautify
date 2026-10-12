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
    fn detects_the_javascript_decorator_helpers_of_esbuild_0_28() {
        // The runtime preamble of `esbuild 0.28.2 --bundle --format=cjs --minify --target=es2016` on standard (JavaScript) decorators.
        let code = r#"var st=Object.create;var j=Object.defineProperty;var S=Object.getOwnPropertyDescriptor;var ot=Object.getOwnPropertyNames;var dt=Object.prototype.hasOwnProperty;var T=(t,r)=>(r=Symbol[t])?r:Symbol.for("Symbol."+t),k=t=>{throw TypeError(t)};var U=(t,r,n)=>r in t?j(t,r,{enumerable:!0,configurable:!0,writable:!0,value:n}):t[r]=n;var P=(t,r)=>j(t,"name",{value:r,configurable:!0});var ft=(t,r)=>{for(var n in r)j(t,n,{get:r[n],enumerable:!0})},lt=(t,r,n,o)=>{if(r&&typeof r=="object"||typeof r=="function")for(let e of ot(r))!dt.call(t,e)&&e!==n&&j(t,e,{get:()=>r[e],enumerable:!(o=S(r,e))||o.enumerable});return t};var gt=t=>lt(j({},"__esModule",{value:!0}),t);var V=t=>{var r;return[,,,st((r=t==null?void 0:t[T("metadata")])!=null?r:null)]},W=["class","method","getter","setter","accessor","field","value","get","set"],I=t=>t!==void 0&&typeof t!="function"?k("Function expected"):t,ht=(t,r,n,o,e)=>({kind:W[t],name:r,metadata:o,addInitializer:s=>n._?k("Already initialized"):e.push(I(s||null))}),mt=(t,r)=>U(r,T("metadata"),t[3]),l=(t,r,n,o)=>{for(var e=0,s=t[r>>1],c=s&&s.length;e<c;e++)r&1?s[e].call(n):o=s[e].call(n,o);return o},h=(t,r,n,o,e,s)=>{var c,f,M,z,y,i=r&7,A=!!(r&8),m=!!(r&16),B=i>3?t.length+1:i?A?1:2:0,N=W[i+5],O=i>3&&(t[B-1]=[]),at=t[B]||(t[B]=[]),g=i&&(!m&&!A&&(e=e.prototype),i<5&&(i>3||!m)&&S(i<4?e:{get[n](){return R(this,s)},set[n](d){return Q(this,s,d)}},n));i?m&&i<4&&P(s,(i>2?"set ":i>1?"get ":"")+n):P(e,n);for(var C=o.length-1;C>=0;C--)z=ht(i,n,M={},t[3],at),i&&(z.static=A,z.private=m,y=z.access={has:m?d=>ct(e,d):d=>n in d},i^3&&(y.get=m?d=>(i^1?R:G)(d,e,i^4?s:g.get):d=>d[n]),i>2&&(y.set=m?(d,E)=>Q(d,e,E,i^4?s:g.set):(d,E)=>d[n]=E)),f=(0,o[C])(i?i<4?m?s:g[N]:i>4?void 0:{get:g.get,set:g.set}:e,z),M._=1,i^4||f===void 0?I(f)&&(i>4?O.unshift(f):i?m?s=f:g[N]=f:e=f):typeof f!="object"||f===null?k("Object expected"):(I(c=f.get)&&(g.get=c),I(c=f.set)&&(g.set=c),I(c=f.init)&&O.unshift(c));return i||mt(t,e),g&&j(e,n,g),m?i^4?s:g:e},X=(t,r,n)=>U(t,typeof r!="symbol"?r+"":r,n),F=(t,r,n)=>r.has(t)||k("Cannot "+n),ct=(t,r)=>Object(r)!==r?k('Cannot use the "in" operator on this value'):t.has(r),R=(t,r,n)=>(F(t,r,"read from private field"),n?n.call(t):r.get(t)),b=(t,r,n)=>r.has(t)?k("Cannot add the same private member more than once"):r instanceof WeakSet?r.add(t):r.set(t,n),Q=(t,r,n,o)=>(F(t,r,"write to private field"),o?o.call(t,n):r.set(t,n),n),G=(t,r,n)=>(F(t,r,"access private method"),n);"#;
        assert_eq!(
            detected(code),
            expect(&[
                ("F", "__accessCheck"),
                ("lt", "__copyProps"),
                ("st", "__create"),
                ("h", "__decorateElement"),
                ("ht", "__decoratorContext"),
                ("mt", "__decoratorMetadata"),
                ("V", "__decoratorStart"),
                ("W", "__decoratorStrings"),
                ("U", "__defNormalProp"),
                ("j", "__defProp"),
                ("I", "__expectFn"),
                ("ft", "__export"),
                ("S", "__getOwnPropDesc"),
                ("ot", "__getOwnPropNames"),
                ("dt", "__hasOwnProp"),
                ("T", "__knownSymbol"),
                ("P", "__name"),
                ("b", "__privateAdd"),
                ("R", "__privateGet"),
                ("ct", "__privateIn"),
                ("G", "__privateMethod"),
                ("Q", "__privateSet"),
                ("X", "__publicField"),
                ("l", "__runInitializers"),
                ("gt", "__toCommonJS"),
                ("k", "__typeError"),
            ])
        );
    }

    #[test]
    fn detects_the_typescript_decorator_helpers_of_esbuild_0_28() {
        // The runtime preamble of `esbuild 0.28.2 --bundle --format=cjs --minify --target=es2016` on TypeScript experimental decorators.
        let code = r#"var l=Object.defineProperty;var g=Object.getOwnPropertyDescriptor;var p=Object.getOwnPropertyNames;var _=Object.prototype.hasOwnProperty;var f=(a,n,r)=>n in a?l(a,n,{enumerable:!0,configurable:!0,writable:!0,value:r}):a[n]=r;var h=(a,n)=>{for(var r in n)l(a,r,{get:n[r],enumerable:!0})},c=(a,n,r,o)=>{if(n&&typeof n=="object"||typeof n=="function")for(let t of p(n))!_.call(a,t)&&t!==r&&l(a,t,{get:()=>n[t],enumerable:!(o=g(n,t))||o.enumerable});return a};var w=a=>c(l({},"__esModule",{value:!0}),a),s=(a,n,r,o)=>{for(var t=o>1?void 0:o?g(n,r):n,y=a.length-1,m;y>=0;y--)(m=a[y])&&(t=(o?m(n,r,t):m(t))||t);return o&&t&&l(n,r,t),t},u=(a,n)=>(r,o)=>n(r,o,a);var i=(a,n,r)=>f(a,typeof n!="symbol"?n+"":n,r);"#;
        assert_eq!(
            detected(code),
            expect(&[
                ("c", "__copyProps"),
                ("s", "__decorateClass"),
                ("u", "__decorateParam"),
                ("f", "__defNormalProp"),
                ("l", "__defProp"),
                ("h", "__export"),
                ("g", "__getOwnPropDesc"),
                ("p", "__getOwnPropNames"),
                ("_", "__hasOwnProp"),
                ("i", "__publicField"),
                ("w", "__toCommonJS"),
            ])
        );
    }

    #[test]
    fn detects_the_using_helpers_of_esbuild_0_28() {
        // The runtime preamble of `esbuild 0.28.2 --bundle --format=cjs --minify --target=es2016` on `using` / `await using`.
        let code = r#"var l=Object.defineProperty;var S=Object.getOwnPropertyDescriptor;var d=Object.getOwnPropertyNames;var f=Object.prototype.hasOwnProperty;var h=(o,s)=>(s=Symbol[o])?s:Symbol.for("Symbol."+o),m=o=>{throw TypeError(o)};var w=(o,s)=>{for(var n in s)l(o,n,{get:s[n],enumerable:!0})},x=(o,s,n,a)=>{if(s&&typeof s=="object"||typeof s=="function")for(let u of d(s))!f.call(o,u)&&u!==n&&l(o,u,{get:()=>s[u],enumerable:!(a=S(s,u))||a.enumerable});return o};var D=o=>x(l({},"__esModule",{value:!0}),o);var g=(o,s,n)=>new Promise((a,u)=>{var t=i=>{try{y(n.next(i))}catch(e){u(e)}},c=i=>{try{y(n.throw(i))}catch(e){u(e)}},y=i=>i.done?a(i.value):Promise.resolve(i.value).then(t,c);y((n=n.apply(o,s)).next())});var p=(o,s,n)=>{if(s!=null){typeof s!="object"&&typeof s!="function"&&m("Object expected");var a,u;n&&(a=s[h("asyncDispose")]),a===void 0&&(a=s[h("dispose")],n&&(u=a)),typeof a!="function"&&m("Object not disposable"),u&&(a=function(){try{u.call(this)}catch(t){return Promise.reject(t)}}),o.push([n,a,s])}else n&&o.push([n]);return s},r=(o,s,n)=>{var a=typeof SuppressedError=="function"?SuppressedError:function(c,y,i,e){return e=Error(i),e.name="SuppressedError",e.error=c,e.suppressed=y,e},u=c=>s=n?new a(c,s,"An error was suppressed during disposal"):(n=!0,c),t=c=>{for(;c=o.pop();)try{var y=c[1]&&c[1].call(c[2]);if(c[0])return Promise.resolve(y).then(t,i=>(u(i),t()))}catch(i){u(i)}if(n)throw s};return t()};"#;
        assert_eq!(
            detected(code),
            expect(&[
                ("g", "__async"),
                ("r", "__callDispose"),
                ("x", "__copyProps"),
                ("l", "__defProp"),
                ("w", "__export"),
                ("S", "__getOwnPropDesc"),
                ("d", "__getOwnPropNames"),
                ("f", "__hasOwnProp"),
                ("h", "__knownSymbol"),
                ("D", "__toCommonJS"),
                ("m", "__typeError"),
                ("p", "__using"),
            ])
        );
    }

    #[test]
    fn detects_the_throwing_using_helpers_of_esbuild_0_19() {
        // The runtime preamble of `esbuild 0.19.12 --bundle --format=cjs --minify --target=es2016`; before 0.21 the helpers throw instead of calling `__typeError`.
        let code = r#"var l=Object.defineProperty;var m=Object.getOwnPropertyDescriptor;var S=Object.getOwnPropertyNames;var d=Object.prototype.hasOwnProperty;var h=(o,s)=>(s=Symbol[o])?s:Symbol.for("Symbol."+o);var f=(o,s)=>{for(var n in s)l(o,n,{get:s[n],enumerable:!0})},w=(o,s,n,u)=>{if(s&&typeof s=="object"||typeof s=="function")for(let a of S(s))!d.call(o,a)&&a!==n&&l(o,a,{get:()=>s[a],enumerable:!(u=m(s,a))||u.enumerable});return o};var x=o=>w(l({},"__esModule",{value:!0}),o);var g=(o,s,n)=>new Promise((u,a)=>{var e=i=>{try{y(n.next(i))}catch(t){a(t)}},c=i=>{try{y(n.throw(i))}catch(t){a(t)}},y=i=>i.done?u(i.value):Promise.resolve(i.value).then(e,c);y((n=n.apply(o,s)).next())});var p=(o,s,n)=>{if(s!=null){if(typeof s!="object"&&typeof s!="function")throw TypeError("Object expected");var u;if(n&&(u=s[h("asyncDispose")]),u===void 0&&(u=s[h("dispose")]),typeof u!="function")throw TypeError("Object not disposable");o.push([n,u,s])}else n&&o.push([n]);return s},r=(o,s,n)=>{var u=typeof SuppressedError=="function"?SuppressedError:function(c,y,i,t){return t=Error(i),t.name="SuppressedError",t.error=c,t.suppressed=y,t},a=c=>s=n?new u(c,s,"An error was suppressed during disposal"):(n=!0,c),e=c=>{for(;c=o.pop();)try{var y=c[1]&&c[1].call(c[2]);if(c[0])return Promise.resolve(y).then(e,i=>(a(i),e()))}catch(i){a(i)}if(n)throw s};return e()};"#;
        assert_eq!(
            detected(code),
            expect(&[
                ("g", "__async"),
                ("r", "__callDispose"),
                ("w", "__copyProps"),
                ("l", "__defProp"),
                ("f", "__export"),
                ("m", "__getOwnPropDesc"),
                ("S", "__getOwnPropNames"),
                ("d", "__hasOwnProp"),
                ("h", "__knownSymbol"),
                ("x", "__toCommonJS"),
                ("p", "__using"),
            ])
        );
    }

    #[test]
    fn detects_the_async_generator_helpers_of_esbuild_0_28() {
        // The runtime preamble of `esbuild 0.28.2 --bundle --format=cjs --minify --target=es2016` on async generators and `for await`.
        let code = r#"var x=Object.defineProperty;var P=Object.getOwnPropertyDescriptor;var R=Object.getOwnPropertyNames;var b=Object.prototype.hasOwnProperty;var f=(e,n)=>(n=Symbol[e])?n:Symbol.for("Symbol."+e),j=e=>{throw TypeError(e)};var k=(e,n)=>{for(var i in n)x(e,i,{get:n[i],enumerable:!0})},q=(e,n,i,a)=>{if(n&&typeof n=="object"||typeof n=="function")for(let t of R(n))!b.call(e,t)&&t!==i&&x(e,t,{get:()=>n[t],enumerable:!(a=P(n,t))||a.enumerable});return e};var z=e=>q(x({},"__esModule",{value:!0}),e);var G=(e,n,i)=>new Promise((a,t)=>{var s=o=>{try{u(i.next(o))}catch(c){t(c)}},r=o=>{try{u(i.throw(o))}catch(c){t(c)}},u=o=>o.done?a(o.value):Promise.resolve(o.value).then(s,r);u((i=i.apply(e,n)).next())}),w=function(e,n){this[0]=e,this[1]=n},g=(e,n,i)=>{var a=(u,o,c,l)=>{try{var d=i[u](o),p=(o=d.value)instanceof w,h=d.done;Promise.resolve(p?o[0]:o).then(y=>p?a(u==="return"?u:"next",o[1]?{done:y.done,value:y.value}:y,c,l):c({value:y,done:h})).catch(y=>a("throw",y,c,l))}catch(y){l(y)}},t=(u,o,c,l)=>r[u]=d=>(o=new Promise((p,h,y)=>(y=()=>a(u,d,p,h),s?s.then(y):y())),l=()=>s===c&&(s=0),s=c=o.then(l,l),o),s,r={};return i=i.apply(e,n),r[f("asyncIterator")]=()=>r,t("next"),t("throw"),t("return"),r},m=e=>{var n=e[f("asyncIterator")],i=!1,a,t={};return n==null?(n=e[f("iterator")](),a=s=>t[s]=r=>n[s](r)):(n=n.call(e),a=s=>t[s]=r=>{if(i){if(i=!1,s==="throw")throw r;return r}return i=!0,{done:!1,value:new w(new Promise(u=>{var o=n[s](r);o instanceof Object||j("Object expected"),u(o)}),1)}}),t[f("iterator")]=()=>t,a("next"),"throw"in n?a("throw"):t.throw=s=>{throw s},"return"in n&&a("return"),t},v=(e,n,i)=>(n=e[f("asyncIterator")])?n.call(e):(e=e[f("iterator")](),n={},i=(a,t)=>(t=e[a])&&(n[a]=s=>new Promise((r,u,o)=>(s=t.call(e,s),o=s.done,Promise.resolve(s.value).then(c=>r({value:c,done:o}),u)))),i("next"),i("return"),n);"#;
        assert_eq!(
            detected(code),
            expect(&[
                ("G", "__async"),
                ("g", "__asyncGenerator"),
                ("w", "__await"),
                ("q", "__copyProps"),
                ("x", "__defProp"),
                ("k", "__export"),
                ("v", "__forAwait"),
                ("P", "__getOwnPropDesc"),
                ("R", "__getOwnPropNames"),
                ("b", "__hasOwnProp"),
                ("f", "__knownSymbol"),
                ("z", "__toCommonJS"),
                ("j", "__typeError"),
                ("m", "__yieldStar"),
            ])
        );
    }

    #[test]
    fn detects_the_super_helpers_of_esbuild_0_28() {
        // The runtime preamble of `esbuild 0.28.2 --bundle --format=cjs --minify --target=es2016` on `super` in lowered async methods.
        let code = r#"var l=Object.defineProperty;var d=Object.getOwnPropertyDescriptor;var g=Object.getOwnPropertyNames;var o=Object.getPrototypeOf,R=Object.prototype.hasOwnProperty;var b=Reflect.get,f=Reflect.set;var j=(s,r)=>{for(var t in r)l(s,t,{get:r[t],enumerable:!0})},k=(s,r,t,e)=>{if(r&&typeof r=="object"||typeof r=="function")for(let n of g(r))!R.call(s,n)&&n!==t&&l(s,n,{get:()=>r[n],enumerable:!(e=d(r,n))||e.enumerable});return s};var q=s=>k(l({},"__esModule",{value:!0}),s);var u=(s,r,t)=>b(o(s),t,r),i=(s,r,t,e)=>(f(o(s),t,e,r),e),m=(s,r,t)=>({get _(){return u(s,r,t)},set _(e){i(s,r,t,e)}});var w=(s,r,t)=>new Promise((e,n)=>{var v=a=>{try{x(t.next(a))}catch(y){n(y)}},p=a=>{try{x(t.throw(a))}catch(y){n(y)}},x=a=>a.done?e(a.value):Promise.resolve(a.value).then(v,p);x((t=t.apply(s,r)).next())});"#;
        assert_eq!(
            detected(code),
            expect(&[
                ("w", "__async"),
                ("k", "__copyProps"),
                ("l", "__defProp"),
                ("j", "__export"),
                ("d", "__getOwnPropDesc"),
                ("g", "__getOwnPropNames"),
                ("o", "__getProtoOf"),
                ("R", "__hasOwnProp"),
                ("b", "__reflectGet"),
                ("f", "__reflectSet"),
                ("u", "__superGet"),
                ("i", "__superSet"),
                ("m", "__superWrapper"),
                ("q", "__toCommonJS"),
            ])
        );
    }

    #[test]
    fn detects_the_private_wrapper_of_esbuild_0_28() {
        // The runtime preamble of `esbuild 0.28.2 --bundle --format=cjs --minify --target=es2016` on `this.#x++` and `[this.#x] = …`.
        let code = r#"var b=Object.defineProperty;var R=Object.getOwnPropertyDescriptor;var d=Object.getOwnPropertyNames;var f=Object.prototype.hasOwnProperty;var l=s=>{throw TypeError(s)};var j=(s,t)=>{for(var i in t)b(s,i,{get:t[i],enumerable:!0})},k=(s,t,i,r)=>{if(t&&typeof t=="object"||typeof t=="function")for(let a of d(t))!f.call(s,a)&&a!==i&&b(s,a,{get:()=>t[a],enumerable:!(r=R(t,a))||r.enumerable});return s};var q=s=>k(b({},"__esModule",{value:!0}),s);var g=(s,t,i)=>t.has(s)||l("Cannot "+i),v=(s,t)=>Object(t)!==t?l('Cannot use the "in" operator on this value'):s.has(t),c=(s,t,i)=>(g(s,t,"read from private field"),i?i.call(s):t.get(s)),o=(s,t,i)=>t.has(s)?l("Cannot add the same private member more than once"):t instanceof WeakSet?t.add(s):t.set(s,i),x=(s,t,i,r)=>(g(s,t,"write to private field"),r?r.call(s,i):t.set(s,i),i);var u=(s,t,i,r)=>({set _(a){x(s,t,a,i)},get _(){return c(s,t,r)}});"#;
        assert_eq!(
            detected(code),
            expect(&[
                ("g", "__accessCheck"),
                ("k", "__copyProps"),
                ("b", "__defProp"),
                ("j", "__export"),
                ("R", "__getOwnPropDesc"),
                ("d", "__getOwnPropNames"),
                ("f", "__hasOwnProp"),
                ("o", "__privateAdd"),
                ("c", "__privateGet"),
                ("v", "__privateIn"),
                ("x", "__privateSet"),
                ("u", "__privateWrapper"),
                ("q", "__toCommonJS"),
                ("l", "__typeError"),
            ])
        );
    }

    #[test]
    fn detects_function_expression_helpers_of_esbuild_0_14() {
        // The runtime preamble of `esbuild 0.14.54 --bundle --format=cjs --minify --target=es2016 --supported:for-of=false`, which emits the runtime as `function` expressions.
        let code = r#"var B=Object.defineProperty;var j=Object.getOwnPropertyDescriptor;var k=Object.getOwnPropertyNames;var q=Object.prototype.hasOwnProperty;var y=function(s,t){for(var i in t)B(s,i,{get:t[i],enumerable:!0})},z=function(s,t,i,r){if(t&&typeof t=="object"||typeof t=="function")for(var o=k(t),l=0,d=o.length,e;l<d;l++)e=o[l],!q.call(s,e)&&e!==i&&B(s,e,{get:function(f){return t[f]}.bind(null,e),enumerable:!(r=j(t,e))||r.enumerable});return s};var A=function(s){return z(B({},"__esModule",{value:!0}),s)};var v=function(s,t,i){if(!t.has(s))throw TypeError("Cannot "+i)},w=function(s,t){if(Object(t)!==t)throw TypeError('Cannot use the "in" operator on this value');return s.has(t)},c=function(s,t,i){return v(s,t,"read from private field"),i?i.call(s):t.get(s)},x=function(s,t,i){if(t.has(s))throw TypeError("Cannot add the same private member more than once");t instanceof WeakSet?t.add(s):t.set(s,i)},b=function(s,t,i,r){return v(s,t,"write to private field"),r?r.call(s,i):t.set(s,i),i},u=function(s,t,i,r){return{set _(o){b(s,t,o,i)},get _(){return c(s,t,r)}}};"#;
        assert_eq!(
            detected(code),
            expect(&[
                ("v", "__accessCheck"),
                ("z", "__copyProps"),
                ("B", "__defProp"),
                ("y", "__export"),
                ("j", "__getOwnPropDesc"),
                ("k", "__getOwnPropNames"),
                ("q", "__hasOwnProp"),
                ("x", "__privateAdd"),
                ("c", "__privateGet"),
                ("w", "__privateIn"),
                ("b", "__privateSet"),
                ("u", "__privateWrapper"),
                ("A", "__toCommonJS"),
            ])
        );
    }

    #[test]
    fn detects_the_binary_loader_helper_of_esbuild_0_28() {
        // The runtime preamble of `esbuild 0.28.2 --bundle --format=cjs --minify --target=es2016 --platform=browser --loader:.bin=binary`.
        let code = r#"var i=Object.defineProperty;var l=Object.getOwnPropertyDescriptor;var u=Object.getOwnPropertyNames;var x=Object.prototype.hasOwnProperty;var y=(o,r)=>{for(var t in r)i(o,t,{get:r[t],enumerable:!0})},A=(o,r,t,m)=>{if(r&&typeof r=="object"||typeof r=="function")for(let a of u(r))!x.call(o,a)&&a!==t&&i(o,a,{get:()=>r[a],enumerable:!(m=l(r,a))||m.enumerable});return o};var R=o=>A(i({},"__esModule",{value:!0}),o);var s=Uint8Array.fromBase64||(()=>{for(var o=new Uint8Array(128),r=0;r<64;r++)o[r<26?r+65:r<52?r+71:r<62?r-4:r*4-205]=r;return t=>{for(var m=t.length,a=new Uint8Array((m-(t[m-1]=="=")-(t[m-2]=="="))*3/4|0),e=0,f=0;e<m;){var c=o[t.charCodeAt(e++)],n=o[t.charCodeAt(e++)],p=o[t.charCodeAt(e++)],d=o[t.charCodeAt(e++)];a[f++]=c<<2|n>>4,a[f++]=n<<4|p>>2,a[f++]=p<<6|d}return a}})();"#;
        assert_eq!(
            detected(code),
            expect(&[
                ("A", "__copyProps"),
                ("i", "__defProp"),
                ("y", "__export"),
                ("l", "__getOwnPropDesc"),
                ("u", "__getOwnPropNames"),
                ("x", "__hasOwnProp"),
                ("s", "__toBinary"),
                ("R", "__toCommonJS"),
            ])
        );
    }

    #[test]
    fn detects_the_binary_loader_helper_of_esbuild_0_14() {
        // The runtime preamble of `esbuild 0.14.54 --bundle --format=cjs --minify --platform=browser --loader:.bin=binary`, before `Uint8Array.fromBase64`.
        let code = r#"var y=Object.create;var f=Object.defineProperty;var A=Object.getOwnPropertyDescriptor;var R=Object.getOwnPropertyNames;var $=Object.getPrototypeOf,g=Object.prototype.hasOwnProperty;var h=(o,r)=>()=>(r||o((r={exports:{}}).exports,r),r.exports),j=(o,r)=>{for(var t in r)f(o,t,{get:r[t],enumerable:!0})},s=(o,r,t,m)=>{if(r&&typeof r=="object"||typeof r=="function")for(let a of R(r))!g.call(o,a)&&a!==t&&f(o,a,{get:()=>r[a],enumerable:!(m=A(r,a))||m.enumerable});return o};var k=(o,r,t)=>(t=o!=null?y($(o)):{},s(r||!o||!o.__esModule?f(t,"default",{value:o,enumerable:!0}):t,o)),q=o=>s(f({},"__esModule",{value:!0}),o);var b=(()=>{for(var o=new Uint8Array(128),r=0;r<64;r++)o[r<26?r+65:r<52?r+71:r<62?r-4:r*4-205]=r;return t=>{for(var m=t.length,a=new Uint8Array((m-(t[m-1]=="=")-(t[m-2]=="="))*3/4|0),e=0,i=0;e<m;){var u=o[t.charCodeAt(e++)],n=o[t.charCodeAt(e++)],p=o[t.charCodeAt(e++)],x=o[t.charCodeAt(e++)];a[i++]=u<<2|n>>4,a[i++]=n<<4|p>>2,a[i++]=p<<6|x}return a}})();"#;
        assert_eq!(
            detected(code),
            expect(&[
                ("h", "__commonJS"),
                ("s", "__copyProps"),
                ("y", "__create"),
                ("f", "__defProp"),
                ("j", "__export"),
                ("A", "__getOwnPropDesc"),
                ("R", "__getOwnPropNames"),
                ("$", "__getProtoOf"),
                ("g", "__hasOwnProp"),
                ("b", "__toBinary"),
                ("q", "__toCommonJS"),
                ("k", "__toESM"),
            ])
        );
    }

    #[test]
    fn detects_the_rest_key_helper_of_esbuild_0_28() {
        // The runtime preamble of `esbuild 0.28.2 --bundle --format=cjs --minify --target=es2016` on object rest with a computed key.
        let code = r#"var c=Object.defineProperty,k=Object.defineProperties,q=Object.getOwnPropertyDescriptor,v=Object.getOwnPropertyDescriptors,z=Object.getOwnPropertyNames,o=Object.getOwnPropertySymbols;var b=Object.prototype.hasOwnProperty,d=Object.prototype.propertyIsEnumerable;var R=(t,e,r)=>e in t?c(t,e,{enumerable:!0,configurable:!0,writable:!0,value:r}):t[e]=r,f=(t,e)=>{for(var r in e||(e={}))b.call(e,r)&&R(t,r,e[r]);if(o)for(var r of o(e))d.call(e,r)&&R(t,r,e[r]);return t},i=(t,e)=>k(t,v(e));var j=t=>typeof t=="symbol"?t:t+"",u=(t,e)=>{var r={};for(var s in t)b.call(t,s)&&e.indexOf(s)<0&&(r[s]=t[s]);if(t!=null&&o)for(var s of o(t))e.indexOf(s)<0&&d.call(t,s)&&(r[s]=t[s]);return r};var S=(t,e)=>{for(var r in e)c(t,r,{get:e[r],enumerable:!0})},g=(t,e,r,s)=>{if(e&&typeof e=="object"||typeof e=="function")for(let n of z(e))!b.call(t,n)&&n!==r&&c(t,n,{get:()=>e[n],enumerable:!(s=q(e,n))||s.enumerable});return t};var h=t=>g(c({},"__esModule",{value:!0}),t);"#;
        assert_eq!(
            detected(code),
            expect(&[
                ("g", "__copyProps"),
                ("R", "__defNormalProp"),
                ("c", "__defProp"),
                ("k", "__defProps"),
                ("S", "__export"),
                ("q", "__getOwnPropDesc"),
                ("v", "__getOwnPropDescs"),
                ("z", "__getOwnPropNames"),
                ("o", "__getOwnPropSymbols"),
                ("b", "__hasOwnProp"),
                ("u", "__objRest"),
                ("d", "__propIsEnum"),
                ("j", "__restKey"),
                ("i", "__spreadProps"),
                ("f", "__spreadValues"),
                ("h", "__toCommonJS"),
            ])
        );
    }
    #[test]
    fn detects_the_wrappers_esbuild_emits_without_object_accessors() {
        // `internal/runtime/runtime.go` builds `__privateWrapper` and `__superWrapper` with
        // `__defProp({}, '_', …)` when the target lacks object accessors, and `__toBinaryNode`
        // falls back to `Buffer` when `Uint8Array.fromBase64` is missing.
        let code = r#"var d=Object.defineProperty,o=Object.getPrototypeOf,b=Reflect.get,f=Reflect.set;var l=s=>{throw TypeError(s)};var g=(s,t,i)=>t.has(s)||l("Cannot "+i),c=(s,t,i)=>(g(s,t,"read from private field"),i?i.call(s):t.get(s)),x=(s,t,i,r)=>(g(s,t,"write to private field"),r?r.call(s,i):t.set(s,i),i);var u=(s,t,i,r)=>d({},"_",{set:a=>x(s,t,a,i),get:()=>c(s,t,r)});var p=(s,r,t)=>b(o(s),t,r),q=(s,r,t,e)=>(f(o(s),t,e,r),e),m=(s,r,t)=>d({},"_",{get:()=>p(s,r,t),set:e=>q(s,r,t,e)});var n=Uint8Array.fromBase64||(t=>new Uint8Array(Buffer.from(t,"base64")));"#;
        assert_eq!(
            detected(code),
            expect(&[
                ("d", "__defProp"),
                ("o", "__getProtoOf"),
                ("b", "__reflectGet"),
                ("f", "__reflectSet"),
                ("l", "__typeError"),
                ("g", "__accessCheck"),
                ("c", "__privateGet"),
                ("x", "__privateSet"),
                ("u", "__privateWrapper"),
                ("p", "__superGet"),
                ("q", "__superSet"),
                ("m", "__superWrapper"),
                ("n", "__toBinaryNode"),
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
