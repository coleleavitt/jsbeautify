use super::*;
use crate::ast_deobfuscate::bun::{BunHelper, BunRuntimeCollector};
use crate::ast_deobfuscate::{DeobfuscateState, semantic};
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_span::SourceType;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};
use rustc_hash::FxHashSet;

/// esbuild's minified `__export` helper, bound to `__export`.
const EXPORT_HELPER: &str =
    "var __export = (e, t) => { for (var n in t) Object.defineProperty(e, n, { get: t[n], enumerable: !0 }); };\n";

fn collect<'a>(alloc: &'a Allocator, code: &'a str) -> (oxc_ast::ast::Program<'a>, ExportNameMap) {
    let parsed = Parser::new(alloc, code, SourceType::mjs()).parse();
    let mut prog = parsed.program;
    let scoping = semantic::scoping(&prog);
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, alloc);
    let mut helpers = BunRuntimeCollector::new();
    traverse_mut_with_ctx(&mut helpers, &mut prog, &mut ctx);
    let export_helpers = helpers
        .helper_symbols()
        .iter()
        .filter(|(_, helper)| **helper == BunHelper::Export)
        .map(|(symbol, _)| *symbol)
        .collect();
    let mut collector = ExportNameCollector::new(export_helpers);
    traverse_mut_with_ctx(&mut collector, &mut prog, &mut ctx);
    (prog, collector.into_map())
}

fn collect_names(code: &str) -> ExportNameMap {
    let alloc = Allocator::default();
    let code = format!("{EXPORT_HELPER}{code}");
    collect(&alloc, &code).1
}

fn collect_and_rename(code: &str) -> (String, usize) {
    let alloc = Allocator::default();
    let code = format!("{EXPORT_HELPER}{code}");
    let (mut prog, map) = collect(&alloc, &code);
    let scoping = semantic::scoping(&prog);
    let reserved: FxHashSet<&str> = scoping
        .root_unresolved_references()
        .keys()
        .map(oxc_ast::ast::Ident::as_str)
        .collect();
    let map = make_names_unique(map, &scoping, &reserved);

    let mut renamer = ExportNameRenamer::new(map);
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &alloc);
    traverse_mut_with_ctx(&mut renamer, &mut prog, &mut ctx);

    (Codegen::new().build(&prog).code, renamer.total_renamed())
}

#[test]
fn extracts_export_names() {
    let code = r"
            var UK, Ba4;
            __export(target, {
                writeToStdout: () => UK,
                exitWithError: () => Ba4
            });
        ";
    let map = collect_names(code);
    assert!(map.values().any(|name| name == "writeToStdout"));
    assert!(map.values().any(|name| name == "exitWithError"));
}

#[test]
fn skips_already_named() {
    let code = r"
            __export(target, {
                foo: () => foo,
                bar: () => bar
            });
        ";
    let map = collect_names(code);
    assert!(map.is_empty(), "Should skip when mangled == original");
}

#[test]
fn skips_generic_names() {
    let code = r"
            var XYZ;
            __export(target, { then: () => XYZ });
        ";
    let map = collect_names(code);
    assert!(map.is_empty(), "Should skip 'then'");
}

#[test]
fn names_default_exports_after_the_esbuild_module_comment() {
    let code = r"
            // src/Button.ts
            var d = {};
            __export(d, { default: () => i, size: () => h });
            function i(t) { return t; }
            var h = 3;
            // src/anon.ts
            var x = {};
            __export(x, { default: () => y });
            var y = { a: 1 };
        ";
    let (out, _) = collect_and_rename(code);
    assert!(out.contains("default: () => Button_default"), "{out}");
    assert!(out.contains("function Button_default(t)"), "{out}");
    assert!(out.contains("var size = 3"), "{out}");
    assert!(out.contains("var anon_default = { a: 1 }"), "{out}");
}

#[test]
fn numbers_default_exports_of_modules_without_identity() {
    let code = r#"
            var d = {};
            __export(d, { default: () => i });
            function i() {}
            var p = {};
            __export(p, { default: () => c });
            var c;
            c = class extends Error { constructor() { super("x"), this.name = "CodecError"; } };
            var x = {};
            __export(x, { default: () => y });
            var y = function encodePng() {};
        "#;
    let (out, _) = collect_and_rename(code);
    assert!(out.contains("function default_1()"), "{out}");
    assert!(
        out.contains("CodecError = class extends Error"),
        "this.name in a joined constructor statement: {out}"
    );
    assert!(
        out.contains("var encodePng$1 = function encodePng()"),
        "the expression's own binding reserves its name: {out}"
    );
}

#[test]
fn default_export_names_never_override_better_names() {
    let code = r"
            var n = {};
            __export(n, { Foo: () => a, default: () => a, default2: () => b });
            var a = 1, b = 2;
            var m = {};
            __export(m, { default: () => readable });
            var readable = 3;
        ";
    let (out, _) = collect_and_rename(code);
    assert!(out.contains("var Foo = 1"), "the named export wins: {out}");
    assert!(
        out.contains("var readable = 3"),
        "an unminified local keeps its name: {out}"
    );
}

#[test]
fn default_export_names_avoid_existing_bindings() {
    let code = r"
            var default_1 = 0;
            var d = {};
            __export(d, { default: () => i });
            function i() { return default_1; }
        ";
    let (out, _) = collect_and_rename(code);
    assert!(out.contains("function default_1$1()"), "{out}");
    assert!(out.contains("return default_1;"), "{out}");
}

#[test]
fn names_native_default_export_locals() {
    let (out, _) = collect_and_rename("const n = [1]; export { n as default };");
    assert!(out.contains("const default_export = [1]"), "{out}");
    assert!(out.contains("export { default_export as default }"), "{out}");

    let (out, _) = collect_and_rename("export default function n() { return n; }");
    assert!(out.contains("export default function default_export()"), "{out}");
    assert!(out.contains("return default_export;"), "{out}");

    let (out, _) = collect_and_rename("class t { constructor() { this.name = \"Parser\"; } } export default t;");
    assert!(out.contains("class Parser"), "{out}");
    assert!(out.contains("export default Parser"), "{out}");
}

#[test]
fn renames_throughout_program() {
    let code = r#"
            __export(target, { writeToStdout: () => UK });
            function UK(data) { process.stdout.write(data); }
            UK("hello");
        "#;
    let (out, count) = collect_and_rename(code);
    assert!(count >= 3, "Should rename decl + refs, got {count}");
    assert!(
        out.contains("function writeToStdout("),
        "Should rename function decl: {out}"
    );
    assert!(out.contains("writeToStdout(\"hello\")"), "Should rename call: {out}");
}

#[test]
fn follows_the_helper_by_symbol_under_any_name() {
    let code = r"
            var Ao = (e, t) => { for (var n in t) Object.defineProperty(e, n, { get: t[n], enumerable: !0 }); };
            Ao(target, { fetchData: () => abc });
            var abc = async () => {};
        ";
    let map = collect_names(code);
    assert!(map.values().any(|name| name == "fetchData"));
}

#[test]
fn ignores_calls_through_a_shadowing_binding() {
    let code = r"
            function f(__export) {
                var abc;
                __export(target, { fetchData: () => abc });
            }
        ";
    assert!(collect_names(code).is_empty());
}

#[test]
fn ignores_a_lookalike_that_is_not_the_helper() {
    let code = r"
            function MR(a, b) { return [a, b]; }
            var abc;
            MR(target, { fetchData: () => abc });
        ";
    assert!(collect_names(code).is_empty());
}

#[test]
fn finds_export_calls_inside_sequences() {
    let code = r"
            var abc, xyz;
            (__export(a, { fetchData: () => abc }), __export(b, { sendData: () => xyz }));
        ";
    let map = collect_names(code);
    assert!(map.values().any(|name| name == "fetchData"));
    assert!(map.values().any(|name| name == "sendData"));
}

#[test]
fn export_rename_does_not_capture_a_global() {
    let code = r#"
            var H = 1;
            __export(target, { fetch: () => H });
            fetch("x");
            H;
        "#;
    let (out, _count) = collect_and_rename(code);
    assert!(out.contains("var fetch$1 = 1"), "{out}");
    assert!(out.contains("fetch(\"x\")"), "global call must stay global: {out}");
    assert!(out.contains("fetch$1;"), "{out}");
}

#[test]
fn handles_large_export_block() {
    let code = r"
            var Fq, XB, Y0, Sg, pE;
            __export(D6, {
                string: () => Fq,
                number: () => XB,
                boolean: () => Y0,
                array: () => Sg,
                object: () => pE
            });
        ";
    let map = collect_names(code);
    assert_eq!(map.len(), 5);
    assert!(map.values().any(|name| name == "string"));
    assert!(map.values().any(|name| name == "number"));
    assert!(map.values().any(|name| name == "boolean"));
}

#[test]
fn only_renames_resolved_export_symbol() {
    let code = r"
            function exported(c) { return c; }
            __export(target, { logEvent: () => exported });
            function unrelated(c) { return c + 1; }
            exported(1);
            unrelated(2);
        ";
    let (out, _count) = collect_and_rename(code);
    assert!(
        out.contains("function logEvent(c)"),
        "exported function should rename: {out}"
    );
    assert!(out.contains("logEvent(1)"), "exported call should rename: {out}");
    assert!(
        out.contains("function unrelated(c)"),
        "unrelated parameter must stay c: {out}"
    );
    assert!(
        out.contains("return c + 1") || out.contains("return c+1"),
        "unrelated body must stay c: {out}"
    );
}

#[test]
fn export_rename_avoids_existing_descendant_binding() {
    let code = r"
            var H;
            __export(target, { writeToStdout: () => H });
            function outer() {
                return function() {
                    let writeToStdout = 1;
                    return H + writeToStdout;
                };
            }
        ";
    let (out, _count) = collect_and_rename(code);
    assert!(
        out.contains("writeToStdout$1"),
        "export rename should be suffixed: {out}"
    );
    assert!(
        out.contains("return writeToStdout$1 + writeToStdout") || out.contains("return writeToStdout$1+writeToStdout"),
        "outer reference must not bind to descendant local: {out}"
    );
}

#[test]
fn export_rename_sanitizes_non_identifier_export_keys() {
    let code = r#"
            var H;
            __export(target, { "foo-bar": () => H });
            H();
        "#;
    let (out, count) = collect_and_rename(code);
    assert!(count >= 2, "Should rename declaration and call: {out}");
    assert!(out.contains("var foo_bar"), "Should sanitize binding name: {out}");
    assert!(out.contains("foo_bar()"), "Should sanitize reference name: {out}");

    let alloc = Allocator::default();
    let parsed = Parser::new(&alloc, &out, SourceType::mjs()).parse();
    assert!(parsed.diagnostics.is_empty(), "sanitized output must parse: {out}");
}

#[test]
fn export_rename_sanitizes_numeric_export_keys() {
    let code = r"
            var H;
            __export(target, { 123: () => H });
            H();
        ";
    let (out, _count) = collect_and_rename(code);
    assert!(out.contains("var _123"), "Should prefix numeric key: {out}");
    assert!(
        out.contains("_123()"),
        "Should rename references to prefixed key: {out}"
    );
}
