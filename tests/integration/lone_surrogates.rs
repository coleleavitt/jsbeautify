//! Lone surrogates in string values: deobfuscation and beautification keep
//! every string's UTF-16 value, so the output behaves like the input under
//! node and never prints U+FFFD in place of a surrogate.

use std::io::Write;
use std::process::{Command, Stdio};

use jsbeautify::{AstDeobfuscator, Options, beautify};

/// Prints each string as its UTF-16 code units, so stdout shows a lone
/// surrogate instead of the U+FFFD node writes for it.
const SHOW: &str = "var show = function (s) { var r = []; for (var i = 0; i < s.length; i++) r.push(s.charCodeAt(i).toString(16)); return r.join(','); };\n";

const CASES: &[(&str, &str)] = &[
    (
        "literals",
        r#"var a = "\uD800", b = 'x\uDFFFy', c = "\uD83D" + "\uDE00";
console.log(show(a), show(b), show(c), c === "\uD83D\uDE00");"#,
    ),
    (
        "templates",
        r#"var t1 = `${"\uDC00"}`, t2 = `a\uD800b${1 + 1}`, t3 = String.raw`\uD800`, t4 = `\uDBFF` + `\uDFFF`;
console.log(show(t1), show(t2), show(t3), show(t4));"#,
    ),
    (
        "keys",
        r#"var o = { "\uD800": 1, b: 2 }, k = { ["\uDBFF"]: 3, ["a" + "\uD800"]: 4 };
class C { "\uD800"() { return 5; } }
console.log(Object.keys(o).map(show).join(" "), o["\uD800"], k["\uDBFF"], k["a\uD800"], new C()["\uD800"]());"#,
    ),
    (
        "concatenation",
        r#"var x = "q", y = "\uDBFF";
var s = ["a\uD800" + "b", "".concat("\uD800", x), "\uD800".concat(x, "z"), y + "\uDFFF", `${y}\uDFFF`];
console.log(s.map(show).join(" "), "a\uD800b".split("\uD800").length);"#,
    ),
    (
        "string array",
        r#"var _0x1a2b = ["\uD800abc", "log", "\uDC00"];
var _0x4e5f = function (_0x1) { _0x1 = _0x1 - 0x0; return _0x1a2b[_0x1]; };
console[_0x4e5f("0x1")](show(_0x4e5f("0x0")), show(_0x4e5f("0x2")));"#,
    ),
    (
        "inlining",
        r#"var d = { a: function () { return "\uDC00"; }, "\uD800": function () { return 2; } };
function f() { return "\uD800"; }
var seq = "1\uD8000".split("\uD800"), i = 0, out = [];
while (true) { switch (seq[i++]) { case "0": out.push(0); continue; case "1": out.push(1); continue; } break; }
console.log(show(d.a()), d["\uD800"](), show(f() + f()), out.join(","));"#,
    ),
];

/// The stdout of `code` run by node, or `None` when node is not installed.
fn node_stdout(code: &str) -> Option<String> {
    let mut child = Command::new("node")
        .arg("-")
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
fn lone_surrogates_keep_their_values() {
    for (name, body) in CASES {
        let src = format!("{SHOW}{body}\n");
        let outputs = [
            ("beautify", beautify(&src, &Options::default())),
            ("deobfuscate", AstDeobfuscator::new().deobfuscate(&src)),
            (
                "deobfuscate --isolated",
                AstDeobfuscator::new().with_isolated(true).deobfuscate(&src),
            ),
        ];
        let expected = node_stdout(&src);
        for (mode, out) in outputs {
            let out = out.unwrap_or_else(|e| panic!("{mode} {name}: {e}"));
            assert!(!out.contains('\u{FFFD}'), "{mode} {name} printed U+FFFD:\n{out}");
            if let Some(expected) = &expected {
                assert_eq!(
                    node_stdout(&out).as_ref(),
                    Some(expected),
                    "{mode} {name} behaves differently:\n{out}"
                );
            }
        }
    }
}

/// An export name must be well-formed Unicode, so the parser rejects this
/// module and every mode reports an error instead of panicking.
#[test]
fn lone_surrogate_export_name_is_an_error() {
    let src = "var x = 1;\nexport { x as \"\\uD800\" };\n";
    assert!(beautify(src, &Options::default()).is_err());
    assert!(AstDeobfuscator::new().deobfuscate(src).is_err());
}
