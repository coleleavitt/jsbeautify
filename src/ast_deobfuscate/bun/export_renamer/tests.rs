use super::*;
use crate::ast_deobfuscate::DeobfuscateState;
use crate::ast_deobfuscate::bun::{BunHelper, BunRuntimeCollector};
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};
use rustc_hash::FxHashSet;

/// esbuild's minified `__export` helper, bound to `__export`.
const EXPORT_HELPER: &str =
    "var __export = (e, t) => { for (var n in t) Object.defineProperty(e, n, { get: t[n], enumerable: !0 }); };\n";

fn collect<'a>(alloc: &'a Allocator, code: &'a str) -> (oxc_ast::ast::Program<'a>, ExportNameMap) {
    let parsed = Parser::new(alloc, code, SourceType::mjs()).parse();
    let mut prog = parsed.program;
    let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
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
    let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
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
            __export(target, { default: () => XYZ });
        ";
    let map = collect_names(code);
    assert!(map.is_empty(), "Should skip 'default'");
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
