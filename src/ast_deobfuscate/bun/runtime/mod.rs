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
