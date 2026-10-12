//! Deobfuscator integration tests over the obfuscated fixtures.

use js_beautify_rs::{AstDeobfuscator, ParseGoal};
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

#[test]
fn statement_splitting_reaches_nested_minifier_output() {
    let src = "function f(x, y) { x && (y ? g() : h()); y || (g(), h()); return x ? (g(), 1) : y ? 2 : 3; }";
    let mut deob = AstDeobfuscator::new();
    let out = deob.deobfuscate(src).expect("deobfuscate nested statement idioms");
    let expected = "function f(v0, v1) {\n\tif (v0) {\n\t\tif (v1) {\n\t\t\tg();\n\t\t} else {\n\t\t\th();\n\t\t}\n\t}\n\tif (!v1) {\n\t\tg();\n\t\th();\n\t}\n\tif (v0) {\n\t\tg();\n\t\treturn 1;\n\t}\n\treturn v1 ? 2 : 3;\n}\n";
    assert_eq!(out.trim_end(), expected.trim_end());
}

#[test]
fn preserves_string_contents_with_confusable_and_zero_width_characters() {
    let src = "var r = new RegExp(\"[\\u038E-\\u03A1]\");\nvar z = \"a\\u200Cb\\u200B\\uFEFF\\u0410\";\nf(r, z);\n";
    let mut deob = AstDeobfuscator::new();
    let out = deob.deobfuscate(src).expect("deobfuscate unicode strings");
    for ch in ['\u{038E}', '\u{03A1}', '\u{200C}', '\u{200B}', '\u{FEFF}', '\u{0410}'] {
        assert!(out.contains(ch), "U+{:04X} must survive: {out}", ch as u32);
    }
}

fn deobfuscate_cjs(code: &str) -> String {
    AstDeobfuscator::new()
        .with_parse_goal(ParseGoal::CommonJs)
        .deobfuscate(code)
        .unwrap_or_else(|e| panic!("deobfuscate failed: {e}\n{code}"))
}

#[test]
fn bun_helper_rename_leaves_shadowing_param_alone() {
    let out = deobfuscate_cjs(
        "var e = Object.create; function f(e) { return e + 1; } module.exports = f; console.log(f(1), typeof e);",
    );
    assert!(out.contains("typeof __create"), "helper should be renamed: {out}");
    assert!(
        !out.contains("return __create"),
        "shadowing param must keep its own binding: {out}"
    );
}

#[test]
fn bun_helper_rename_avoids_capture_by_inner_binding() {
    let out =
        deobfuscate_cjs("var e = Object.create; function g(__create) { return e(__create); } module.exports = g;");
    assert!(
        !out.contains("__create(__create)"),
        "inner param must not capture the helper: {out}"
    );
}

#[test]
fn bun_helper_rename_avoids_capturing_a_global() {
    let out = deobfuscate_cjs("var e = Object.create; function g() { return __create; } module.exports = [e, g];");
    assert!(
        out.contains("return __create;"),
        "free reference must stay global: {out}"
    );
    assert!(
        !out.contains("var __create ="),
        "helper must not shadow the global: {out}"
    );
}

#[test]
fn bun_helper_rename_skips_eval_visible_binding() {
    let out = deobfuscate_cjs("var e = Object.create; function g() { return eval('typeof e'); } console.log(g());");
    assert!(out.contains("var e = Object.create"), "eval reads `e` by name: {out}");
}

#[test]
fn optional_computed_member_keeps_optional_chain() {
    let out = deobfuscate_cjs(r#"const u = {}; const k = "p"; console.log(u[k]?.["city"] ?? "-", u.p?.["a"]?.["b"]);"#);
    assert!(out.contains("]?.city ?? \"-\""), "got: {out}");
    assert!(out.contains(".p?.a?.b"), "got: {out}");
}

#[test]
fn class_names_from_this_name_never_collide() {
    let out = deobfuscate_cjs(
        "var a=class extends Error{constructor(m){super(m),this.name=\"AbortError\"}},\
         b=class extends Error{constructor(m){super(m),this.name=\"AbortError\"}};\
         function f(){return AbortError}module.exports={a,b,f};",
    );
    assert!(
        out.contains("AbortError$1 = class") && out.contains("AbortError$2 = class"),
        "{out}"
    );
    assert!(out.contains("return AbortError;"), "the global read stays free: {out}");
}
