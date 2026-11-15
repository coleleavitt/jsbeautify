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
fn keeps_calls_and_aliases_a_with_object_may_redirect() {
    // In the `with` body `t` and `_0xdec` may read `o`'s properties, and `w`
    // may hold `o._0xdec`, so only the last call is decoded.
    let (output, pass) = run_with_prelude(
        "
        const t = _0xdec;
        with (o) { t(0x101); (function () { return _0xdec(0x102); })(); var w = _0xdec; }
        console.log(w(0x100), t(0x100));
        ",
    );
    assert_eq!(pass.inlined(), 1, "{output}");
    assert_eq!(pass.removed_rotations(), 0, "{output}");
    assert!(output.contains("with(o){t(257);"), "{output}");
    assert!(output.contains("return_0xdec(258)"), "{output}");
    assert!(output.contains(r#"console.log(w(256),"1")"#), "{output}");
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
fn countdown_too_long_to_finish_bails() {
    let code = LEGACY.replace("_0xr(++_0xn)", "_0xr(0x100000001)");
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
        // A sloppy-mode function called without a receiver gets the global object.
        "function f() { this._0xdec = null; } f();",
        "(function () { return this; })()._0xdec = null;",
        // `Function` through `.constructor` runs code in the global scope.
        "(function () {}).constructor('_0xdec = null')();",
        "[]['constructor']['constructor']('_0xdec = null')();",
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
            function f() {{ 'use strict'; this._0xdec = 1; }}
            function g() {{ this.other = 1; }}
            class C {{ x = this._0xdec; m() {{ this._0xdec = 1; }} static {{ this._0xdec = 1; }} }}
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

/// Helpers whose bodies decode; the names say nothing.
const XOR_HELPER: &str = "function h1(s, k) { var r = ''; for (var i = 0; i < s.length; i++) r += String.fromCharCode(s.charCodeAt(i) ^ k.charCodeAt(i % k.length)); return r; }\n";
const XOR_NUMBER_HELPER: &str = "function h2(s, k) { let r = '', i; for (i = 0; i < s.length; ++i) { r = r + String.fromCharCode(k ^ s.charCodeAt(i)); } return r; }\n";
const RC4_HELPER: &str = "function h3(d, k) { var s = [], j = 0, x, r = ''; d = atob(d);
  for (var i = 0; i < 256; i++) s[i] = i;
  for (i = 0; i < 256; i++) { j = (j + s[i] + k.charCodeAt(i % k.length)) % 256; x = s[i]; s[i] = s[j]; s[j] = x; }
  i = 0; j = 0;
  for (var y = 0; y < d.length; y++) { i = (i + 1) % 256; j = (j + s[i]) % 256; x = s[i]; s[i] = s[j]; s[j] = x; r += String.fromCharCode(d.charCodeAt(y) ^ s[(s[i] + s[j]) % 256]); }
  return r; }\n";

/// A variable array read by `_0xdec`, whose body returns `ret`.
fn decoder_program(array: &str, helpers: &str, ret: &str, uses: &str) -> String {
    format!("var _0xarr = [{array}];\n{helpers}function _0xdec(i) {{ return {ret}; }}\n{uses}\n")
}

#[test]
fn decodes_helpers_recognised_by_their_bodies() {
    // "HELLO" XOR 0x01, "secret" RC4 with "password", "hello world" in base64.
    let (output, pass) = run(&decoder_program(
        "'IDMMN', 'jJBbfClK', 'aGVsbG8gd29ybGQ='",
        "",
        "atob(_0xarr[i])",
        "f(_0xdec(2));",
    ));
    assert_eq!(pass.inlined(), 1, "{output}");
    // `squash` drops the space.
    assert!(output.contains(r#"f("helloworld")"#), "{output}");

    let (output, pass) = run(&decoder_program(
        "'IDMMN'",
        XOR_HELPER,
        "h1(_0xarr[i], '\\x01')",
        "f(_0xdec(0));",
    ));
    assert_eq!(pass.inlined(), 1, "{output}");
    assert!(output.contains(r#"f("HELLO")"#), "{output}");

    let (output, pass) = run(&decoder_program(
        "'IDMMN'",
        XOR_NUMBER_HELPER,
        "h2(_0xarr[i], 1)",
        "f(_0xdec(0));",
    ));
    assert_eq!(pass.inlined(), 1, "{output}");
    assert!(output.contains(r#"f("HELLO")"#), "{output}");

    let (output, pass) = run(&decoder_program(
        "'jJBbfClK'",
        RC4_HELPER,
        "h3(_0xarr[i], 'password')",
        "f(_0xdec(0));",
    ));
    assert_eq!(pass.inlined(), 1, "{output}");
    assert!(output.contains(r#"f("secret")"#), "{output}");
}

#[test]
fn misleading_helper_names_are_not_decoded() {
    for helper in [
        "function xorDecode(s, k) { return s + k; }\n",
        "function rc4Decode(s, k) { return s; }\n",
        // One operator away from the XOR loop.
        "function xorDecode(s, k) { var r = ''; for (var i = 0; i < s.length; i++) r += String.fromCharCode(s.charCodeAt(i) + k.charCodeAt(i % k.length)); return r; }\n",
        // The loop writes a binding outside the helper.
        "var i; function xorDecode(s, k) { var r = ''; for (i = 0; i < s.length; i++) r += String.fromCharCode(s.charCodeAt(i) ^ k.charCodeAt(i % k.length)); return r; }\n",
    ] {
        let name = if helper.contains("rc4") {
            "rc4Decode"
        } else {
            "xorDecode"
        };
        let (output, pass) = run(&decoder_program(
            "'IDMMN'",
            helper,
            &format!("{name}(_0xarr[i], 'key')"),
            "f(_0xdec(0));",
        ));
        assert_eq!(pass.inlined(), 0, "{helper}:\n{output}");
    }
}

#[test]
fn rc4_helper_with_an_outer_temporary_is_not_decoded() {
    let helper = RC4_HELPER.replace("var s = [], j = 0, x, r = '';", "var s = [], j = 0, r = '';");
    let (output, pass) = run(&decoder_program(
        "'jJBbfClK'",
        &format!("var x;\n{helper}"),
        "h3(_0xarr[i], 'password')",
        "f(_0xdec(0));",
    ));
    assert_eq!(pass.inlined(), 0, "{output}");
}

#[test]
fn local_atob_is_not_decoded() {
    for prelude in [
        "function atob(s) { return s; }\n",
        "var atob = function (s) { return s; };\n",
        "{ function atob(s) { return s; } }\n",
        "atob = function (s) { return s; };\n",
    ] {
        let (output, pass) = run(&decoder_program("'aGk='", prelude, "atob(_0xarr[i])", "f(_0xdec(0));"));
        assert_eq!(pass.inlined(), 0, "{prelude}:\n{output}");
    }
    // A parameter named `atob` shadows the global inside the decoder.
    let (output, pass) =
        run("var _0xarr = ['aGk='];\nfunction _0xdec(i, atob) { return atob(_0xarr[i]); }\nf(_0xdec(0));");
    assert_eq!(pass.inlined(), 0, "{output}");
    // A local `atob` at the call site does not change the decoder's.
    let (output, pass) = run(&decoder_program(
        "'aGk='",
        "",
        "atob(_0xarr[i])",
        "function g(atob) { return _0xdec(0); }",
    ));
    assert_eq!(pass.inlined(), 1, "{output}");
}

#[test]
fn reassigned_or_block_shadowed_helper_bails() {
    for uses in ["f(_0xdec(0)); h1 = null;", "f(_0xdec(0)); { function h1() {} }"] {
        let (output, pass) = run(&decoder_program("'IDMMN'", XOR_HELPER, "h1(_0xarr[i], '\\x01')", uses));
        assert!(!pass.changed(), "{uses}:\n{output}");
    }
}

#[test]
fn reads_inline_offsets_and_combined_offsets() {
    let (output, pass) = run("
        var _0xarr = ['a', 'b', 'c'];
        function d1(i) { return _0xarr[i - 0x100]; }
        function d2(i) { i = i - 0x10; return _0xarr[i + 1]; }
        f(d1(0x101), d1('0x102'), d2(0x10), d2('0x11'));
    ");
    assert_eq!(pass.inlined(), 4, "{output}");
    assert!(output.contains(r#"f("b","c","b","c")"#), "{output}");
}

#[test]
fn string_index_concatenated_by_plus_is_not_converted() {
    let (output, pass) = run("
        var _0xarr = ['a', 'b'];
        function d(i) { return _0xarr[i + 1]; }
        f(d('0'), d(0));
    ");
    assert_eq!(pass.inlined(), 1, "{output}");
    assert!(output.contains(r#"f(d("0"),"b")"#), "{output}");
}

#[test]
fn inlines_element_reads_of_a_decoded_array_nothing_changes() {
    let (output, pass) = run("
        var arr = ['hello', 'world', 'log'];
        function d(i) { return arr[i]; }
        console[d(2)](arr[0], (arr[1]), arr[0x1].length, arr.length);
    ");
    assert_eq!(pass.inlined(), 4, "{output}");
    assert!(
        output.contains(r#"console["log"]("hello","world","world".length,arr.length);"#),
        "{output}"
    );
}

#[test]
fn element_reads_of_a_changed_or_escaping_array_bail() {
    for uses in [
        "arr[0] = 'z'; f(arr[0]);",
        "delete arr[0]; f(arr[1]);",
        "arr.reverse(); f(arr[0]);",
        "g(arr); f(arr[0]);",
        "f(arr[0](), arr[1]);",
        // Properties the array does not own may be accessors that get the
        // array as `this`.
        "f(arr[7], arr[1]);",
        "f(arr[i], arr[1]);",
        "f(arr.evil, arr[1]);",
        "f(arr[i](), arr[1]);",
        "f((arr.shift)(), arr[1]);",
        "f(new arr[0](), arr[1]);",
        "f(arr[0]``, arr[1]);",
        "f(arr?.[0], arr[1]);",
        "for (arr[0] in o); f(arr[1]);",
        "arr = []; f(arr[0]);",
    ] {
        let (output, pass) = run(&format!(
            "var arr = ['a', 'b'];\nfunction d(i) {{ return arr[i]; }}\n{uses}"
        ));
        assert!(!pass.changed(), "{uses}:\n{output}");
    }
}

#[test]
fn element_reads_in_a_with_body_stay() {
    let (output, pass) =
        run("var arr = ['a', 'b']; function d(i) { return arr[i]; } with (o) { f(arr[0]); } f(arr[1]);");
    assert_eq!(pass.inlined(), 1, "{output}");
    assert!(output.contains(r#"with(o){f(arr[0]);}f("b");"#), "{output}");
}

#[test]
fn rotation_counts_down_to_one() {
    // `while (--k)` from 2 rotates once.
    let (output, pass) = run("
        var _0x1111 = ['a', 'b', 'c'];
        (function (_0x2222, _0x3333) {
            var _0x4444 = function (_0x5555) { while (--_0x5555) { _0x2222.push(_0x2222.shift()); } };
            _0x4444(2);
        })(_0x1111, 0x123);
        function _0xdec(a) { return _0x1111[a]; }
        f(_0xdec(0), _0x1111[2]);
    ");
    assert_eq!(pass.inlined(), 2, "{output}");
    assert_eq!(pass.removed_rotations(), 1, "{output}");
    assert!(output.contains(r#"f("b","a")"#), "{output}");
}

#[test]
fn unproven_rotation_bails() {
    for rotation in [
        // Rotates in a loop it cannot count.
        "(function (a, n) { while (n--) a.push(a.shift()); })(_0xarr, 2);",
        // A countdown followed by more work on the array.
        "(function (a, n) { var r = function (k) { while (--k) { a.push(a.shift()); } }; r(n); a.reverse(); })(_0xarr, 2);",
        // Two rotations.
        "(function (a, n) { var r = function (k) { while (--k) { a.push(a.shift()); } }; r(n); })(_0xarr, 2);
         (function (a, n) { var r = function (k) { while (--k) { a.push(a.shift()); } }; r(n); })(_0xarr, 3);",
    ] {
        let (output, pass) = run(&format!(
            "var _0xarr = ['a', 'b', 'c'];\n{rotation}\nfunction _0xdec(i) {{ return _0xarr[i]; }}\nf(_0xdec(0), _0xarr[1]);"
        ));
        assert!(!pass.changed(), "{rotation}:\n{output}");
    }
}

#[test]
fn several_decoders_share_one_array() {
    let (output, pass) = run("
        var _0xarr = ['aGk=', 'b'];
        function plain(i) { return _0xarr[i]; }
        function base64(i) { i = i - 1; return atob(_0xarr[i + 1]); }
        f(plain(1), base64(0), _0xarr[1]);
    ");
    assert_eq!(pass.inlined(), 3, "{output}");
    assert!(output.contains(r#"f("b","hi","b")"#), "{output}");
}

#[test]
fn decoding_follows_javascript() {
    use super::decoding::Decoding;
    let apply = |decoding: Decoding, value: &str| decoding.apply(value).map(std::borrow::Cow::into_owned);
    assert_eq!(apply(Decoding::Base64, "w6k="), Some("\u{c3}\u{a9}".to_string()));
    // `atob` accepts these, but reads them differently from strict base64.
    assert_eq!(apply(Decoding::Base64, "aGk"), None);
    assert_eq!(apply(Decoding::Base64, "aGl="), None);
    assert_eq!(apply(Decoding::XorString(Vec::new()), "abc"), Some("abc".to_string()));
    assert_eq!(
        apply(Decoding::XorString(vec![0xd800]), "\u{d7ff}"),
        Some("\u{fff}".to_string())
    );
    // A result with a lone surrogate has no Rust string.
    assert_eq!(apply(Decoding::XorString(vec![0xd800]), "a"), None);
    assert_eq!(apply(Decoding::XorNumber(1), "IDMMN"), Some("HELLO".to_string()));
    assert_eq!(
        apply(
            Decoding::Rc4 {
                key: "password".encode_utf16().collect(),
                base64: true
            },
            "jJBbfClK"
        ),
        Some("secret".to_string())
    );
    assert_eq!(
        apply(
            Decoding::Rc4 {
                key: Vec::new(),
                base64: false
            },
            "x"
        ),
        None
    );
}

#[test]
fn repeated_parameter_names_bail() {
    // The second `i` binds the second argument, so `_0xdec(0)` reads `undefined`.
    let (output, pass) = run("var _0xarr = ['a', 'b'];\nfunction _0xdec(i, i) { return _0xarr[i]; }\nf(_0xdec(0));");
    assert!(!pass.changed(), "{output}");
    // `a` is the number, so the rotation throws.
    let (output, pass) = run("var _0xarr = ['a', 'b'];
        (function (a, a) { var r = function (k) { while (--k) { a.push(a.shift()); } }; r(2); })(_0xarr, 1);
        function _0xdec(i) { return _0xarr[i]; }
        f(_0xdec(0));");
    assert!(!pass.changed(), "{output}");
}

#[test]
fn wrapper_function_properties_are_not_elements() {
    assert_untouched("console.log(_0xarr[0], _0xdec(0x100));");
}

#[test]
fn decodes_arrays_and_decoders_local_to_a_function() {
    let (output, pass) = run("
        function main() {
            var _0x1a2b = ['Hello', 'log'];
            (function (a, n) { var r = function (k) { while (--k) { a.push(a.shift()); } }; r(++n); })(_0x1a2b, 0);
            function _0xdec(i) { i = i - 0x64; return _0x1a2b[i]; }
            const alias = _0xdec;
            console[_0xdec(0x65)](alias(0x64), _0x1a2b[1]);
        }
        const arrow = () => { var a = ['x', 'y']; function d(i) { return a[i]; } return a[1]; };
    ");
    assert_eq!(pass.inlined(), 4, "{output}");
    assert_eq!(pass.removed_rotations(), 1, "{output}");
    // `r(++n)` counts down from 1, so the array is not rotated.
    assert!(output.contains(r#"console["log"]("Hello","log");"#), "{output}");
    assert!(output.contains(r#"return"y";"#), "{output}");
    assert!(!output.contains("alias"), "{output}");
}

#[test]
fn function_with_parameter_expressions_is_left_alone() {
    // A default value cannot see the body's declarations, and it runs first.
    let (output, pass) =
        run("function main(x = 1) { var a = ['x']; function d(i) { return a[i]; } return d(0) + a[0]; }");
    assert!(!pass.changed(), "{output}");
}

#[test]
fn direct_eval_in_a_nested_function_bails() {
    let (output, pass) = run(
        "function main() { var a = ['x']; function d(i) { return a[i]; } (function () { eval('1'); })(); return d(0); }",
    );
    assert!(!pass.changed(), "{output}");
}

#[test]
fn script_direct_reads_bail_on_global_object_writes() {
    for body in [
        "(function () { this.arr.reverse(); })();",
        "[].constructor.constructor('arr.reverse()')();",
    ] {
        let (output, pass) = run_as(
            &format!("var arr = ['x', 'y'];\nfunction d(i) {{ return arr[i]; }}\n{body}\nf(arr[0]);"),
            SourceType::script(),
        );
        assert!(!pass.changed(), "{body}:\n{output}");
    }
}

#[test]
fn function_inside_with_is_left_alone() {
    // `atob` and the decoder may read the `with` object's properties.
    let (output, pass) =
        run("with (o) { (function () { var a = ['Zm9v']; function d(i) { return atob(a[i]); } f(d(0), a[0]); })(); }");
    assert!(!pass.changed(), "{output}");
}

#[test]
fn plain_string_arrays_stay_as_written() {
    // No decoder reads them and no rotation reorders them, so they are not
    // obfuscation, however their elements are read.
    let (output, pass) = run("
        var days = ['Mon', 'Tue'];
        f(days[0], days.length);
        function g() { const names = ['a', 'b']; return names[1]; }
    ");
    assert!(!pass.changed(), "{output}");
}

#[test]
fn rotated_array_without_a_decoder_inlines_its_reads() {
    let (output, pass) = run("
        var _0xarr = ['a', 'b', 'c'];
        (function (a, n) { var r = function (k) { while (--k) { a.push(a.shift()); } }; r(++n); })(_0xarr, 1);
        f(_0xarr[0], _0xarr[2]);
    ");
    assert_eq!(pass.inlined(), 2, "{output}");
    assert_eq!(pass.removed_rotations(), 1, "{output}");
    assert!(output.contains(r#"f("b","a")"#), "{output}");
}
