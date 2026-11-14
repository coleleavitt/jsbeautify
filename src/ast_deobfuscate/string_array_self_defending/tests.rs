use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_span::SourceType;

use super::SelfDefendingStringArray;
use super::checksum::{parse_int, string_to_number};
use crate::ast_deobfuscate::{DeobfuscateState, semantic};

/// `code` without whitespace, so assertions do not depend on the layout.
fn squash(code: &str) -> String {
    code.split_whitespace().collect()
}

fn run(code: &str) -> (String, SelfDefendingStringArray) {
    run_as(code, SourceType::cjs())
}

fn run_as(code: &str, source_type: SourceType) -> (String, SelfDefendingStringArray) {
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, code, source_type).parse();
    assert!(ret.diagnostics.is_empty(), "parse errors: {:?}", ret.diagnostics);
    let mut program = ret.program;
    let scoping = semantic::scoping(&program);
    let mut ctx = oxc_traverse::ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
    let mut pass = SelfDefendingStringArray::new();
    oxc_traverse::traverse_mut_with_ctx(&mut pass, &mut program, &mut ctx);
    (squash(&Codegen::new().build(&program).code), pass)
}

/// After one rotation the array is `['1', '2', 'a']`, so `_0xdec(0x100)`
/// reads `'1'` and `_0xdec(0x102)` reads `'a'`.
const PRELUDE: &str = "
function _0xarr() { const a = ['a', '1', '2']; _0xarr = function () { return a; }; return _0xarr(); }
function _0xdec(i, k) { i = i - 0x100; const s = _0xarr(); let v = s[i]; return v; }
(function (g, t) {
    const d = _0xdec, a = g();
    while (!![]) {
        try { const c = parseInt(d(0x100)); if (c === t) break; else a['push'](a['shift']()); }
        catch (e) { a['push'](a['shift']()); }
    }
}(_0xarr, 1));
";

fn run_with_prelude(body: &str) -> (String, SelfDefendingStringArray) {
    run(&format!("{PRELUDE}{body}"))
}

fn assert_untouched(body: &str) {
    let (output, pass) = run_with_prelude(body);
    assert!(!pass.changed(), "expected no change:\n{output}");
}

#[test]
fn obfuscator_fixture_decodes_every_call() {
    let code = include_str!("../../../tests/fixtures/obfuscator/string_array_default.js");
    let (output, pass) = run(code);
    assert_eq!(pass.inlined(), 8, "{output}");
    assert_eq!(pass.removed_rotations(), 1, "{output}");
    assert!(!output.contains("parseInt"), "{output}");
    // Only the decoder's own declaration is left.
    assert_eq!(output.matches("_0x4079(").count(), 1, "{output}");
    assert!(
        output.contains(r#"console["log"](greet("world"),shout("quiet"))"#),
        "{output}"
    );
    assert!(
        output.contains(r#"[_0x1fbe0c["greeting"],_0x1fbe0c["target"],"from","obfuscator"]["join"]"#),
        "{output}"
    );
    assert!(output.contains(r#"_0x1c2328["toUpperCase"]()"#), "{output}");
}

#[test]
fn decodes_direct_and_aliased_calls_after_rotation() {
    let (output, pass) = run_with_prelude(
        "
        const top = _0xdec;
        function f() { const local = top; const inner = local; return inner(0x102) + _0xdec(0x100); }
        console.log(f(), top(0x101), (() => top(0x102))());
        ",
    );
    assert_eq!(pass.inlined(), 4, "{output}");
    assert_eq!(pass.removed_rotations(), 1, "{output}");
    assert!(output.contains(r#"return"a"+"1";"#), "{output}");
    assert!(output.contains(r#"console.log(f(),"2",(()=>"a")());"#), "{output}");
    assert!(!output.contains("top") && !output.contains("local"), "{output}");
}

#[test]
fn decodes_calls_in_every_expression_position() {
    let (output, pass) = run_with_prelude(
        "
        const t = _0xdec;
        class C { [t(0x100)] = t(0x101); static [t(0x102)]() {} }
        const o = { [t(0x100)]: t(0x101), ...t(0x102) };
        const f = (x = t(0x100)) => `${t(0x101)}`;
        String.raw`${t(0x102)}`;
        t(0x100)?.length;
        a?.[t(0x101)](t(0x102));
        for (const k in t(0x100)) {}
        module.exports = [t(0x101), ...t(0x102)];
        ",
    );
    assert_eq!(pass.removed_rotations(), 1, "{output}");
    assert!(!output.contains("t("), "{output}");
    assert_eq!(output.matches("_0xdec(").count(), 1, "{output}");
}

#[test]
fn decodes_without_a_rotation() {
    let (output, pass) = run("
        function _0xarr() { const a = ['x', 'y']; _0xarr = function () { return a; }; return _0xarr(); }
        function _0xdec(i) { i = i - 0x10; const s = _0xarr(); return s[i]; }
        console.log(_0xdec(0x11));
    ");
    assert_eq!(pass.inlined(), 1, "{output}");
    assert!(output.contains(r#"console.log("y")"#), "{output}");
}

#[test]
fn keeps_calls_it_cannot_prove_and_then_the_rotation() {
    let (output, pass) = run_with_prelude(
        "
        console.log(_0xdec(0x103), _0xdec(n), _0xdec(0x100, 1), _0xdec(0x101));
        with (o) { _0xdec(0x101); }
        ",
    );
    assert_eq!(pass.inlined(), 1, "{output}");
    assert_eq!(pass.removed_rotations(), 0, "{output}");
    assert!(
        output.contains(r#"_0xdec(259),_0xdec(n),_0xdec(256,1),"2""#),
        "{output}"
    );
}

#[test]
fn keeps_an_alias_that_may_be_read_before_its_declaration() {
    let (output, pass) = run_with_prelude(
        "
        function f() { g(); const early = _0xdec; function g() { return early(0x100); } }
        function h(v) { switch (v) { case 0: const s = _0xdec; break; case 1: s(0x100); } }
        let changed = _0xdec; changed = null;
        console.log(changed(0x100));
        ",
    );
    assert_eq!(pass.inlined(), 0, "{output}");
    assert_eq!(pass.removed_rotations(), 0, "{output}");
}

#[test]
fn shadowing_binding_is_not_the_decoder() {
    let (output, pass) = run_with_prelude("function f(_0xdec) { return _0xdec(0x100); }");
    assert_eq!(pass.inlined(), 0, "{output}");
    // The only decoder references left are the rotation's own.
    assert_eq!(pass.removed_rotations(), 1, "{output}");
}

#[test]
fn decodes_through_var_aliases_in_function_bodies() {
    let (output, pass) = run_with_prelude(
        "
        var top = _0xdec;
        function f() { var local = top; if (local) { var inBlock = _0xdec; } return local(0x101) + inBlock(0x101); }
        console.log(f(), top(0x100));
        ",
    );
    assert_eq!(pass.inlined(), 2, "{output}");
    assert!(output.contains(r#"return"2"+inBlock(257);"#), "{output}");
    assert!(output.contains(r#"console.log(f(),"1");"#), "{output}");
}

#[test]
fn block_function_can_overwrite_a_var_alias() {
    let (output, pass) =
        run_with_prelude("function f(c) { var a = _0xdec; if (c) { function a() { return 'x'; } } return a(0x100); }");
    assert_eq!(pass.inlined(), 0, "{output}");
}

#[test]
fn decodes_a_variable_array_rotated_by_a_countdown() {
    let code = include_str!("../../../tests/fixtures/obfuscated/simple_string_array.js");
    let (output, pass) = run(code);
    assert_eq!(pass.inlined(), 3, "{output}");
    assert_eq!(pass.removed_rotations(), 1, "{output}");
    assert!(
        output.contains(r#"console.log("World");console.log("Test");console.log("Message");"#),
        "{output}"
    );
}

/// The older obfuscator layout: a variable array, a countdown rotation by
/// `++n`, and a decoder variable declared after it that takes hex strings.
const LEGACY: &str = r"
var _0x2e3c = ['second', 'third', 'log', 'Hello World!'];
(function (_0xa, _0xn) {
    var _0xr = function (_0xk) { while (--_0xk) { _0xa['push'](_0xa['shift']()); } };
    _0xr(++_0xn);
}(_0x2e3c, 0x6));
var _0x4a2b = function (_0xi, _0xj) { _0xi = _0xi - 0x0; var _0xv = _0x2e3c[_0xi]; return _0xv; };
";

#[test]
fn decodes_the_legacy_layout_with_string_indexes() {
    let (output, pass) = run(&format!(
        "{LEGACY}console[_0x4a2b('0x0')](_0x4a2b('0x1'), _0x4a2b(' 3 '));"
    ));
    assert_eq!(pass.inlined(), 3, "{output}");
    assert_eq!(pass.removed_rotations(), 1, "{output}");
    // `squash` drops the space in "Hello World!".
    assert!(output.contains(r#"console["log"]("HelloWorld!","third");"#), "{output}");
}

#[test]
fn solves_a_checksum_over_string_indexes() {
    let code = PRELUDE.replace("parseInt(d(0x100))", "parseInt(d('0x100'))");
    let (output, pass) = run(&format!("{code}console.log(_0xdec('0x102'));"));
    assert_eq!(pass.inlined(), 1, "{output}");
    assert_eq!(pass.removed_rotations(), 1, "{output}");
    assert!(output.contains(r#"console.log("a");"#), "{output}");
}

#[test]
fn keeps_string_indexes_it_cannot_convert() {
    let (output, pass) = run(&format!("{LEGACY}f(_0x4a2b('1e0'), _0x4a2b('-0x1'), _0x4a2b('0x1'));"));
    assert_eq!(pass.inlined(), 1, "{output}");
    assert_eq!(pass.removed_rotations(), 0, "{output}");
}

#[test]
fn string_index_without_subtraction_is_not_converted() {
    let (output, pass) = run("
        var _0xa = ['x', 'y'];
        function _0xd(i) { return _0xa[i]; }
        f(_0xd('1'), _0xd(1));
    ");
    assert_eq!(pass.inlined(), 1, "{output}");
    assert!(output.contains(r#"f(_0xd("1"),"y")"#), "{output}");
}

#[test]
fn decoder_variable_after_running_code_bails() {
    let code = LEGACY.replace("var _0x4a2b", "f(); var _0x4a2b");
    let (output, pass) = run(&format!("{code}g(_0x4a2b('0x0'));"));
    assert!(!pass.changed(), "{output}");
}

#[test]
fn endless_countdown_bails() {
    let code = LEGACY.replace("_0xr(++_0xn)", "_0xr(0)");
    let (output, pass) = run(&format!("{code}g(_0x4a2b('0x0'));"));
    assert!(!pass.changed(), "{output}");
}

#[test]
fn reassigned_decoder_bails() {
    assert_untouched("_0xdec = function () { return 'z'; }; console.log(_0xdec(0x100));");
}

#[test]
fn escaping_wrapper_bails() {
    assert_untouched("_0xarr().reverse(); console.log(_0xdec(0x100));");
}

#[test]
fn direct_eval_bails() {
    assert_untouched("eval('x'); console.log(_0xdec(0x100));");
}

#[test]
fn code_before_the_rotation_bails() {
    let (output, pass) = run(&format!("console.log(_0xdec(0x100));{PRELUDE}"));
    assert!(!pass.changed(), "{output}");
}

#[test]
fn block_function_named_like_the_decoder_bails() {
    assert_untouched("{ function _0xdec() {} } console.log(_0xdec(0x100));");
}

#[test]
fn unsolvable_checksum_bails() {
    let code = PRELUDE.replace("}(_0xarr, 1));", "}(_0xarr, 7));");
    let (output, pass) = run(&format!("{code}console.log(_0xdec(0x100));"));
    assert!(!pass.changed(), "{output}");
}

#[test]
fn stops_inlining_past_the_size_budget() {
    let long = "x".repeat(300_000);
    let calls = "_0xdec(0x100),".repeat(8);
    let (output, pass) = run(&format!(
        "
        function _0xarr() {{ const a = ['{long}']; _0xarr = function () {{ return a; }}; return _0xarr(); }}
        function _0xdec(i) {{ i = i - 0x100; const s = _0xarr(); return s[i]; }}
        console.log({calls});
        "
    ));
    // The budget is four times the ~300 kB source.
    assert_eq!(pass.inlined(), 4, "{}", output.len());
}

#[test]
fn script_keeps_the_rotation_and_top_level_aliases_other_scripts_see() {
    let code = include_str!("../../../tests/fixtures/obfuscator/string_array_default.js");
    let (output, pass) = run_as(code, SourceType::script());
    assert_eq!(pass.inlined(), 8, "{output}");
    assert_eq!(pass.removed_rotations(), 0, "{output}");
    assert!(output.contains("parseInt"), "{output}");
    assert!(output.contains("const_0x43940f=_0x4079;"), "{output}");
    // Aliases local to a function are not visible to other scripts.
    assert!(
        !output.contains("_0x39a9d7") && !output.contains("_0x5c9b86"),
        "{output}"
    );
}

#[test]
fn script_global_object_writes_bail() {
    for body in [
        "globalThis._0xdec = function () { return 'evil'; };",
        "this._0xdec = null;",
        "window['_0x' + 'dec'] = null;",
        "Object.assign(self, {});",
        "(0, eval)('_0xdec = null');",
        "new Function('_0xdec = null')();",
        "const top_ = _0xdec; window.top_ = null;",
    ] {
        let (output, pass) = run_as(
            &format!("{PRELUDE}{body} console.log(_0xdec(0x100));"),
            SourceType::script(),
        );
        assert!(!pass.changed(), "{body}:\n{output}");
    }
}

#[test]
fn script_harmless_global_object_uses_do_not_bail() {
    let (output, pass) = run_as(
        &format!(
            "{PRELUDE}
            if (typeof window !== 'undefined') window.onload = null;
            function f() {{ this._0xdec = 1; }}
            class C {{ x = this._0xdec; static {{ this._0xdec = 1; }} }}
            console.log(_0xdec(0x100));"
        ),
        SourceType::script(),
    );
    assert_eq!(pass.inlined(), 1, "{output}");
}

#[test]
fn parse_int_follows_javascript() {
    assert_eq!(parse_int("2NCQHev"), Some(2.0));
    assert_eq!(parse_int("  \u{FEFF}42px"), Some(42.0));
    assert_eq!(parse_int("-0x1F"), Some(-31.0));
    assert_eq!(parse_int("+7"), Some(7.0));
    assert_eq!(parse_int("1e5"), Some(1.0));
    assert!(parse_int("abc").is_some_and(f64::is_nan));
    assert!(parse_int("0x").is_some_and(f64::is_nan));
    assert!(parse_int("").is_some_and(f64::is_nan));
    assert!(parse_int("-0").is_some_and(|v| v == 0.0 && v.is_sign_negative()));
    assert_eq!(parse_int("0x20000000000001"), None);
    assert_eq!(parse_int("123456789012345678901"), None);
}

#[test]
fn string_to_number_follows_javascript() {
    assert_eq!(string_to_number("0x1f"), Some(31.0));
    assert_eq!(string_to_number(" 010\n"), Some(10.0));
    assert_eq!(string_to_number(""), Some(0.0));
    assert_eq!(string_to_number("1e3"), None);
    assert_eq!(string_to_number("-1"), None);
    assert_eq!(string_to_number("0x"), None);
}
