//! String-array decoders are matched through symbols and recognised by their
//! bodies, not their names: a call is decoded only when the code it reaches
//! is proven to be the decoder, and the output prints what the input prints
//! under node.

use std::io::Write;
use std::process::{Command, Stdio};

use jsbeautify::AstDeobfuscator;

const XOR: &str = "function q(s, k) { var r = ''; for (var i = 0; i < s.length; i++) r += String.fromCharCode(s.charCodeAt(i) ^ k.charCodeAt(i % k.length)); return r; }\n";
const RC4: &str = "function z(d, k) { var s = [], j = 0, x, r = ''; d = atob(d);
  for (var i = 0; i < 256; i++) s[i] = i;
  for (i = 0; i < 256; i++) { j = (j + s[i] + k.charCodeAt(i % k.length)) % 256; x = s[i]; s[i] = s[j]; s[j] = x; }
  i = 0; j = 0;
  for (var y = 0; y < d.length; y++) { i = (i + 1) % 256; j = (j + s[i]) % 256; x = s[i]; s[i] = s[j]; s[j] = x; r += String.fromCharCode(d.charCodeAt(y) ^ s[(s[i] + s[j]) % 256]); }
  return r; }\n";

/// `_0xdec` reads `_0xarr` and returns `ret`; `body` runs after both.
fn program(array: &str, prelude: &str, ret: &str, body: &str) -> String {
    format!("var _0xarr = [{array}];\n{prelude}function _0xdec(i) {{ return {ret}; }}\n{body}\n")
}

const PRINT: &str = "console.log(JSON.stringify([_0xdec(0), _0xdec(1)]));";

/// `(name, program, decoded)`: `decoded` is the output's print line when the
/// calls must be decoded, `None` when they must stay.
fn cases() -> Vec<(&'static str, String, Option<&'static str>)> {
    vec![
        (
            "renamed XOR helper",
            program(r#""\u0003\u000e\u0007", "\u0010""#, XOR, "q(_0xarr[i], 'key')", PRINT),
            Some(r#"console.log(JSON.stringify(["hk~", "{"]));"#),
        ),
        (
            "renamed RC4 helper, non-ASCII result",
            program(r#""jJBbfClK", "kq6d""#, RC4, "z(_0xarr[i], 'password')", PRINT),
            Some("console.log(JSON.stringify([\"secret\", \"m[\u{a5}\"]));"),
        ),
        (
            "global atob",
            program(r#""w6k=", "aGk=""#, "", "atob(_0xarr[i])", PRINT),
            Some(r#"console.log(JSON.stringify(["Ã©", "hi"]));"#),
        ),
        (
            "helper named xorDecode that concatenates",
            program(
                r#""abc", "def""#,
                "function xorDecode(s, k) { return s + '|' + k; }\n",
                "xorDecode(_0xarr[i], 'key')",
                PRINT,
            ),
            None,
        ),
        (
            "helper named rc4Decode that returns its input",
            program(
                r#""T1dMUVJ3Yz0=", "b""#,
                "function rc4Decode(s, k) { return s; }\n",
                "rc4Decode(_0xarr[i], 'abcdefgh')",
                PRINT,
            ),
            None,
        ),
        (
            "local atob function",
            program(r#""w6k=", "aGk=""#, "function atob(s) { return 'local:' + s; }\n", "atob(_0xarr[i])", PRINT),
            None,
        ),
        (
            "block function atob",
            program(r#""w6k=", "aGk=""#, "{ function atob(s) { return 'block:' + s; } }\n", "atob(_0xarr[i])", PRINT),
            None,
        ),
        (
            "global atob reassigned",
            program(r#""w6k=", "aGk=""#, "atob = function (s) { return 'set:' + s; };\n", "atob(_0xarr[i])", PRINT),
            None,
        ),
        (
            "shadowed decoder",
            program(
                "'a', 'b'",
                "",
                "_0xarr[i]",
                "function f() { function _0xdec(i) { return 'inner' + i; } return _0xdec(0); }\nconsole.log(f(), _0xdec(1));",
            ),
            None,
        ),
        (
            "decoder with a repeated parameter name",
            program(
                "'a', 'b'",
                "",
                "_0xarr[i]",
                "function _0xdup(i, i) { return _0xarr[i]; }\nconsole.log(_0xdup(0), _0xdup(1, 0), _0xdec(1));",
            ),
            None,
        ),
        (
            "decoder reassigned after use",
            program(
                "'a', 'b'",
                "",
                "_0xarr[i]",
                "console.log(_0xdec(0));\n_0xdec = function (i) { return 'changed' + i; };\nconsole.log(_0xdec(1));",
            ),
            None,
        ),
        (
            "helper reassigned after use",
            program(
                r#""\u0003\u000e\u0007""#,
                XOR,
                "q(_0xarr[i], 'key')",
                "console.log(_0xdec(0));\nq = function () { return 'swapped'; };\nconsole.log(_0xdec(0));",
            ),
            None,
        ),
        (
            "block function named like the decoder",
            program(
                "'a', 'b'",
                "",
                "_0xarr[i]",
                "console.log(_0xdec(0));\n{ function _0xdec(i) { return 'block' + i; } }\nconsole.log(_0xdec(1));",
            ),
            None,
        ),
        (
            "with object holding the decoder's and array's names",
            program(
                "'a', 'b'",
                "",
                "_0xarr[i]",
                "var o = { _0xdec: function (i) { return 'with' + i; }, _0xarr: ['x', 'y'] };\nwith (o) { console.log(_0xdec(0), _0xarr[1]); }\nconsole.log(_0xdec(1), _0xarr[0]);",
            ),
            None,
        ),
        (
            "element written",
            program("'a', 'b'", "", "_0xarr[i]", "_0xarr[0] = 'z';\nconsole.log(_0xdec(0), _0xarr[1]);"),
            None,
        ),
        (
            "element read as a method",
            program(
                "'a', 'b'",
                "",
                "_0xarr[i]",
                "Array.prototype[5] = function () { this.push('pushed'); return 'p'; };\nconsole.log(_0xarr[5](), _0xdec(2), _0xarr[0]);",
            ),
            None,
        ),
        (
            "countdown rotation",
            "var _0xarr = ['a', 'b', 'c'];\n(function (a, n) { var r = function (k) { while (--k) { a.push(a.shift()); } }; r(++n); })(_0xarr, 1);\nfunction _0xdec(i) { return _0xarr[i - 0x10]; }\nconsole.log(JSON.stringify([_0xdec(0x10), _0xarr[2]]));\n".to_string(),
            Some(r#"console.log(JSON.stringify(["b", "a"]));"#),
        ),
        (
            "decoder local to a function",
            "function main() { var _0xarr = ['Hello', 'log']; function _0xdec(i) { i = i - 0x64; return _0xarr[i]; } console[_0xdec(0x65)](_0xdec(0x64), _0xarr[0]); }\nmain();\n".to_string(),
            Some(r#"console.log("Hello", "Hello");"#),
        ),
        (
            "local decoder seen by a default parameter",
            "var _0xdec = function () { return 'outer'; };\nfunction main(x = _0xdec(0)) { var _0xarr = ['a']; function _0xdec(i) { return _0xarr[i]; } return x + _0xdec(0); }\nconsole.log(main());\n".to_string(),
            None,
        ),
        (
            "rotation by an unknown loop",
            "var _0xarr = ['a', 'b', 'c'];\n(function (a, n) { while (n--) a.push(a.shift()); })(_0xarr, 2);\nfunction _0xdec(i) { return _0xarr[i]; }\nconsole.log(_0xdec(0), _0xarr[1]);\n".to_string(),
            None,
        ),
    ]
}

/// Classic scripts that reach their globals other than by name. `--isolated`
/// promises they do not, so these run only as scripts.
fn script_cases() -> Vec<(&'static str, String)> {
    vec![
        (
            "array reversed through a sloppy function's this",
            "var _0xarr = ['x', 'y'];\nfunction _0xdec(i) { return _0xarr[i]; }\n(function () { this._0xarr.reverse(); })();\nconsole.log(_0xarr[0], _0xdec(1));\n".to_string(),
        ),
        (
            "array reversed through Function from .constructor",
            "var _0xarr = ['x', 'y'];\nfunction _0xdec(i) { return _0xarr[i]; }\n[].constructor.constructor('_0xarr.reverse()')();\nconsole.log(_0xarr[0], _0xdec(1));\n".to_string(),
        ),
        (
            "array reversed by a prototype getter",
            "var _0xarr = ['x', 'y'];\nfunction _0xdec(i) { return _0xarr[i]; }\nObject.defineProperty(Array.prototype, 'evil', { get() { this.reverse(); } });\n_0xarr.evil;\nconsole.log(_0xarr[0], _0xdec(1));\n".to_string(),
        ),
        (
            "atob read through a with object in a function",
            "var o = { atob: function () { return 'with'; } };\nwith (o) { (function () { var a = ['Zm9v']; function d(i) { return atob(a[i]); } console.log(d(0)); })(); }\n".to_string(),
        ),
    ]
}

/// The stdout of `code` run by node as a classic script, whose top-level
/// bindings are properties of the global object, or `None` when node is not
/// installed.
fn node_stdout(code: &str) -> Option<String> {
    let mut child = Command::new("node")
        .args([
            "-e",
            "require('vm').runInThisContext(require('fs').readFileSync(0, 'utf8'))",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    child
        .stdin
        .take()
        .expect("node stdin")
        .write_all(code.as_bytes())
        .expect("write to node");
    let output = child.wait_with_output().expect("run node");
    assert!(output.status.success(), "node failed on:\n{code}");
    Some(String::from_utf8(output.stdout).expect("utf-8 stdout"))
}

#[test]
fn decoders_are_proven_before_they_are_decoded() {
    for (name, src, decoded) in cases() {
        let expected = node_stdout(&src);
        for isolated in [false, true] {
            let out = AstDeobfuscator::new()
                .with_isolated(isolated)
                .deobfuscate(&src)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            if let Some(line) = decoded {
                assert!(
                    out.contains(line),
                    "{name} (isolated: {isolated}) is not decoded:\n{out}"
                );
            }
            if let Some(expected) = &expected {
                assert_eq!(
                    node_stdout(&out).as_ref(),
                    Some(expected),
                    "{name} (isolated: {isolated}) behaves differently:\n{out}"
                );
            }
        }
    }
}

#[test]
fn scripts_reaching_globals_indirectly_are_not_decoded() {
    for (name, src) in script_cases() {
        let out = AstDeobfuscator::new()
            .deobfuscate(&src)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        if let Some(expected) = node_stdout(&src) {
            assert_eq!(node_stdout(&out), Some(expected), "{name} behaves differently:\n{out}");
        }
    }
}
