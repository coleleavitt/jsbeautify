use super::*;
use crate::ast_deobfuscate::DeobfuscateState;
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

fn collect_names(code: &str) -> ExportNameMap {
    let alloc = Allocator::default();
    let parsed = Parser::new(&alloc, code, SourceType::mjs()).parse();
    let mut prog = parsed.program;
    let mut collector = ExportNameCollector::new();
    let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &alloc);
    traverse_mut_with_ctx(&mut collector, &mut prog, &mut ctx);
    collector.into_map()
}

fn collect_and_rename(code: &str) -> (String, usize) {
    let alloc = Allocator::default();
    let parsed = Parser::new(&alloc, code, SourceType::mjs()).parse();
    let mut prog = parsed.program;

    let mut collector = ExportNameCollector::new();
    let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &alloc);
    traverse_mut_with_ctx(&mut collector, &mut prog, &mut ctx);
    let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
    let map = make_names_unique(collector.into_map(), &scoping);

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
fn handles_j_dollar_form() {
    let code = r"
            j$(target, { fetchData: () => abc });
            var abc = async () => {};
        ";
    let map = collect_names(code);
    assert!(map.values().any(|name| name == "fetchData"));
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
    assert!(parsed.errors.is_empty(), "sanitized output must parse: {out}");
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
