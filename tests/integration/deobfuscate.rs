//! Deobfuscator integration tests over the obfuscated fixtures.

use js_beautify_rs::AstDeobfuscator;
use oxc_allocator::Allocator;
use oxc_parser::Parser;
use oxc_span::SourceType;

use crate::fixtures;

#[test]
fn deobfuscates_string_array_fixture() {
    let src = fixtures::obfuscated("simple_string_array.js");
    let mut deob = AstDeobfuscator::new();
    let out = deob.deobfuscate(&src).expect("deobfuscate string-array sample");
    assert_ne!(out.trim(), "");
}

#[test]
fn deobfuscates_turbopack_runtime_guarded_iife_without_top_level_return() {
    let src = r#"
        (globalThis["TURBOPACK"] || (globalThis["TURBOPACK"] = [])).push([
            "output/chunk.js",
            {"runtimeModuleIds":["[project]/input/index.js [test] (ecmascript)"]}
        ]);
        (() => {
            if (!Array.isArray(globalThis["TURBOPACK"])) {
                return;
            }
            var CHUNK_BASE_PATH = "";
            function startRuntime() {
                return CHUNK_BASE_PATH;
            }
            startRuntime();
        })();
    "#;
    let mut deob = AstDeobfuscator::new();
    let out = deob.deobfuscate(src).expect("deobfuscate Turbopack runtime guard IIFE");

    assert!(
        out.contains("Array.isArray(globalThis.TURBOPACK)") || out.contains("Array.isArray(globalThis[\"TURBOPACK\"])"),
        "Turbopack runtime guard should be preserved: {out}"
    );
    assert!(
        out.contains("return"),
        "guarded return should remain inside the IIFE: {out}"
    );
}

#[test]
fn deobfuscates_typescript_typed_declarators_without_empty_recovery() {
    let src = r"
        function makeHydrator() {
            const foo = 1, hydrator: Hydrator = (value: number): number => value;
            return hydrator(foo);
        }
    ";
    let mut deob = AstDeobfuscator::new();
    let out = deob.deobfuscate(src).expect("deobfuscate TypeScript typed declarator");

    assert!(
        !out.trim().is_empty(),
        "TypeScript input should not collapse to an empty recovered AST"
    );
    assert!(out.contains("hydrator"), "binding should survive: {out}");

    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, &out, SourceType::mjs().with_typescript(true)).parse();
    assert!(
        parsed.diagnostics.is_empty(),
        "deobfuscated TypeScript should re-parse cleanly: {:?}\n{out}",
        parsed.diagnostics.first()
    );
}

#[test]
fn deobfuscator_never_panics_on_any_fixture() {
    for (name, src) in fixtures::all_obfuscated() {
        let mut deob = AstDeobfuscator::new();
        // We don't assert a specific transformation here — only that the
        // deobfuscator handles every sample shape without panicking and
        // returns a result either way.
        let _ = deob
            .deobfuscate(&src)
            .unwrap_or_else(|e| panic!("deobfuscate {name} returned hard error: {e}"));
    }
}
