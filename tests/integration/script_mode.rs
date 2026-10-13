//! Classic-script inputs: top-level bindings are globals other scripts read by
//! name, so `-d` must neither rename nor remove them unless the input is
//! declared isolated.

use jsbeautify::{AstDeobfuscator, ParseGoal};
use oxc_allocator::Allocator;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use std::collections::BTreeSet;

use crate::fixtures;

fn deobfuscate(code: &str, goal: ParseGoal, isolated: bool) -> String {
    AstDeobfuscator::new()
        .with_parse_goal(goal)
        .with_isolated(isolated)
        .deobfuscate(code)
        .unwrap_or_else(|e| panic!("deobfuscate failed: {e}\n{code}"))
}

/// Names bound in the global scope when `code` runs as a classic script.
fn global_names(code: &str) -> BTreeSet<String> {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, code, SourceType::script()).parse();
    assert!(parsed.diagnostics.is_empty(), "not a valid script: {code}");
    let scoping = SemanticBuilder::new().build(&parsed.program).semantic.into_scoping();
    scoping
        .get_bindings(scoping.root_scope_id())
        .keys()
        .map(ToString::to_string)
        .collect()
}

/// One input per pass that renames or deletes top-level bindings in a module.
const PASS_SAMPLES: &[(&str, &str)] = &[
    (
        "dead-var sweep + local slot renamer",
        "var a=!0,b=void 0;function f(n){for(;n;)n--;return a?n:b}var x=1;{let x=2;f(x)}",
    ),
    ("call proxy", "function _0x1(a,b){return a(b)} _0x1(console.log, 1);"),
    (
        "call proxy (var)",
        "var _0x1=function(a,b){return a(b)}; _0x1(console.log, 1);",
    ),
    ("operator proxy", "function add(a,b){return a+b} console.log(add(1,2));"),
    ("unary proxy", "function neg(a){return !a} console.log(neg(1));"),
    ("function inliner", "function dbl(a){return a*2} console.log(dbl(3));"),
    (
        "string array + decoder + variable renamer",
        r#"var _0x5a3b=["Hello","World"];function _0xdec(i){i=i-0;return _0x5a3b[i]}console.log(_0xdec(0),_0xdec(1));"#,
    ),
    ("string-var inlining", r#"var s="a";var t=s+"b";console.log(t);"#),
    ("window alias", "var w=window;w.foo=1;console.log(w.foo);"),
    (
        "unused lexical bindings",
        "let a=1;const b=2;class C{}var unused;function g(){}",
    ),
    (
        "trampoline",
        "var T;T=function(a){return g(a)};function g(a){return a}console.log(T(1));",
    ),
    (
        "Bun module handles",
        r#"var __commonJS=(cb,mod)=>function(){return mod||(0,cb[Object.keys(cb)[0]])((mod={exports:{}}).exports,mod),mod.exports};var X=__commonJS({"a.js"(e,m){m.exports=1}});console.log(X());"#,
    ),
    ("Bun runtime helpers", "var e=Object.create;console.log(typeof e);"),
    (
        "class names",
        r#"var Q=class{constructor(){this.name="RealName"}};new Q;"#,
    ),
    (
        "top-level control-flow unflattener",
        r#"var _flow="1|0".split("|"),_i=0;while(true){switch(_flow[_i++]){case "0":console.log(2);continue;case "1":console.log(1);continue}break}"#,
    ),
];

#[test]
fn script_keeps_every_global_binding() {
    for (pass, code) in PASS_SAMPLES {
        let expected = global_names(code);
        let out = deobfuscate(code, ParseGoal::Auto, false);
        let kept = global_names(&out);
        assert!(
            expected.is_subset(&kept),
            "{pass}: lost globals {:?}\n{out}",
            expected.difference(&kept).collect::<Vec<_>>()
        );
    }
}

#[test]
fn isolated_script_lets_each_pass_fire() {
    for (pass, code) in PASS_SAMPLES {
        let out = deobfuscate(code, ParseGoal::Auto, true);
        assert_ne!(global_names(&out), global_names(code), "{pass} did not fire:\n{out}");
    }
}

#[test]
fn commonjs_top_level_is_private() {
    let out = deobfuscate("var unused = 1; f(); return;", ParseGoal::Auto, false);
    assert!(!out.contains("unused"), "{out}");
    assert!(out.contains("return;"), "{out}");
}

#[test]
fn module_top_level_is_private() {
    let code = "var unused = 1; var used = 2; export { used };";
    let out = deobfuscate(code, ParseGoal::Auto, false);
    assert!(!out.contains("unused"), "{out}");
    assert_eq!(out, deobfuscate(code, ParseGoal::Module, true));
}

#[test]
fn explicit_goal_overrides_detection() {
    let code = "var unused = 1;";
    assert!(deobfuscate(code, ParseGoal::Script, false).contains("unused"));
    assert!(!deobfuscate(code, ParseGoal::Module, false).contains("unused"));
    assert!(!deobfuscate(code, ParseGoal::CommonJs, false).contains("unused"));
}

#[test]
fn global_read_through_window_survives() {
    let out = deobfuscate("var a = 1; console.log(window.a);", ParseGoal::Auto, false);
    assert!(out.contains("var a = 1"), "{out}");
}

#[test]
fn sloppy_script_invalid_as_module_deobfuscates() {
    let code = "var o={p:!0};with(o){var r=p}var n=010;function h(a){arguments[0]=2;return a}console.log(r,n,h(1));";
    let out = deobfuscate(code, ParseGoal::Auto, false);
    assert!(out.contains("with (o)") || out.contains("with(o)"), "{out}");
    assert!(out.contains("arguments[0] = 2"), "{out}");
    assert!(out.contains("p: true"), "{out}");
    assert!(global_names(code).is_subset(&global_names(&out)), "{out}");
}

#[test]
fn annex_b_block_functions_keep_their_names() {
    for code in [
        "{ function f() { return f } } { function f() { return f } } console.log(typeof f());",
        "function g() { { function f() { return 1 } } { function f() { return 2 } } return f(); } console.log(g());",
        "a; { b; function a() {} }",
    ] {
        let out = deobfuscate(code, ParseGoal::Auto, false);
        assert_eq!(
            out.matches("function f()").count(),
            code.matches("function f()").count(),
            "{out}"
        );
        assert_eq!(
            out.matches("function a()").count(),
            code.matches("function a()").count(),
            "{out}"
        );
    }
}

#[test]
fn obfuscated_fixtures_keep_their_globals_as_scripts() {
    for (name, src) in fixtures::all_obfuscated() {
        let out = deobfuscate(&src, ParseGoal::Script, false);
        let expected = global_names(&src);
        assert!(
            expected.is_subset(&global_names(&out)),
            "{name}: lost globals {:?}",
            expected.difference(&global_names(&out)).collect::<Vec<_>>()
        );
    }
}
