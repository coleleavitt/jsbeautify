//! Inlining passes replace a binding's references with what its declaration
//! holds, and often delete the declaration. That is only sound when every
//! reference is a call that reaches the declaration's value:
//!
//! - a direct `eval` can call, reassign or shadow a binding it can see by name
//!   (`with_scope::redirectable_symbols`);
//! - a `var` or `let` holds nothing until its initializer runs
//!   (`call_sites::initialized_early`);
//! - an optional call `f?.()` is not rewritten, so it still needs `f`.

use std::path::Path;
use std::process::Command;

fn deobfuscate(dir: &Path, code: &str, flags: &[&str]) -> String {
    let path = dir.join("input.js");
    std::fs::write(&path, code).expect("write input");
    let output = Command::new(env!("CARGO_BIN_EXE_jsbeautify"))
        .args(flags)
        .arg(&path)
        .output()
        .expect("run jsbeautify");
    assert!(output.status.success(), "jsbeautify {flags:?} failed: {output:?}");
    String::from_utf8(output.stdout).expect("stdout is utf8")
}

/// What `code` prints under node, or `None` when node is not installed.
fn node_stdout(dir: &Path, code: &str) -> Option<String> {
    let path = dir.join("run.js");
    std::fs::write(&path, code).expect("write script");
    let output = match Command::new("node").arg(&path).output() {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => panic!("run node: {error}"),
    };
    assert!(output.status.success(), "node failed: {output:?}");
    Some(String::from_utf8(output.stdout).expect("stdout is utf8"))
}

/// Each input and what node prints for it and for its deobfuscated output.
/// `var bmak, _abck` make the detector run the Akamai forwarder passes.
const CASES: &[(&str, &str)] = &[
    // operator proxy reassigned by eval
    (
        r#"(function () { function BI(a, b) { return a === b; } eval("BI = function (a, b) { return a + b; }"); console.log(BI(1, 2), BI(3, 4)); })();"#,
        "3 7",
    ),
    // operator proxy shadowed by an eval `var` in a nested function
    (
        r#"(function () { var BI = function (a, b) { return a * b; }; function g() { eval("var BI = function (a, b) { return a - b; }"); return BI(7, 2); } console.log(g(), BI(3, 4)); })();"#,
        "5 12",
    ),
    // operator proxy called by name inside eval
    (
        r#"(function () { function BI(a, b) { return a + b; } console.log(BI(1, 2), BI(3, 4), eval("BI(5, 6)")); })();"#,
        "3 7 11",
    ),
    // unary proxy reassigned by eval
    (
        r#"(function () { function Bd(x) { return !x; } eval("Bd = function (x) { return 'e' + x; }"); console.log(Bd(1), Bd(0)); })();"#,
        "e1 e0",
    ),
    // lookup forwarder reassigned by eval (Akamai phase 0.5e)
    (
        r#"var bmak = 1, _abck = 2; (function () { function A() { return ["a", "b", "c"]; } function F(i) { return A()[i]; } eval("F = function (i) { return 'e' + i; }"); console.log(F(0), F(2)); })();"#,
        "e0 e2",
    ),
    // method-call forwarder reassigned by eval (Akamai phase 0.5e2)
    (
        r#"var bmak = 1, _abck = 2; (function () { function M(o, a) { return o.concat(a); } eval("M = function (o, a) { return 'e'; }"); console.log(M([1], 2), M([3], 4)); })();"#,
        "e e",
    ),
    // single-use function called by name inside eval (function_inline)
    (
        r#"(function () { function BI(a, b) { return a + b; } console.log(BI(1, 2), eval("BI(5, 6)")); })();"#,
        "3 11",
    ),
    // single-use function reassigned by eval (function_inline)
    (
        r#"(function () { function BI(a, b) { return a + b; } eval("BI = function (a, b) { return a * b; }"); console.log(BI(3, 4)); })();"#,
        "12",
    ),
    // call proxy reassigned by eval
    (
        r#"(function () { function t(a, b) { return a + b; } function p(a, b) { return t(a, b); } eval("p = function (a, b) { return 'e'; }"); console.log(p(1, 2)); })();"#,
        "e",
    ),
    // trampoline reassigned by eval (Akamai phase 0.5f)
    (
        r#"var bmak = 1, _abck = 2; (function () { var S = 1; function D(s, args) { return "d" + s + args[0]; } var T = function () { return D.apply(this, [S, arguments]); }; eval("T = function (x) { return 'e' + x; }"); console.log(T(5), T(6)); })();"#,
        "e5 e6",
    ),
    // dispatcher object reassigned by eval
    (
        r#"(function () { var d = { a: function () { return 1; }, b: function () { return 2; } }; eval("d = { a: function () { return 'ea'; }, b: function () { return 'eb'; } }"); console.log(d.a(), d.b()); })();"#,
        "ea eb",
    ),
    // window alias reassigned by eval
    (
        r#"(function () { var g = global; eval("g = { Math: { max: function () { return 'e'; } } }"); console.log(g.Math.max(1, 2)); })();"#,
        "e",
    ),
    // operator proxy `var` called before it is set (TypeError)
    (
        r#"(function () { try { console.log(BI(1, 2)); } catch (e) { console.log("TE"); } var BI = function (a, b) { return a + b; }; console.log(BI(2, 2)); })();"#,
        "TE\n4",
    ),
    // unary proxy `var` called before it is set
    (
        r#"(function () { try { console.log(Bd(1)); } catch (e) { console.log("TE"); } var Bd = function (x) { return !x; }; console.log(Bd(0)); })();"#,
        "TE\ntrue",
    ),
    // call proxy `var` called before it is set
    (
        r#"(function () { function t(a, b) { return a + b; } try { console.log(p(1, 2)); } catch (e) { console.log("TE"); } var p = function (a, b) { return t(a, b); }; })();"#,
        "TE",
    ),
    // trampoline `var` called before it is set (Akamai phase 0.5f)
    (
        r#"var bmak = 1, _abck = 2; (function () { var S = 1; function D(s, args) { return "d" + s + args[0]; } try { console.log(T(5)); } catch (e) { console.log("TE"); } var T = function () { return D.apply(this, [S, arguments]); }; console.log(T(6)); })();"#,
        "TE\nd16",
    ),
    // operator proxy called with `?.`
    (
        r"(function () { function add(a, b) { return a + b; } console.log(add?.(1, 2), add(3, 4)); })();",
        "3 7",
    ),
    // unary proxy called with `?.`
    (
        r"(function () { function Bd(x) { return !x; } console.log(Bd?.(1), Bd(0)); })();",
        "false true",
    ),
    // single-use function called with `?.`
    (
        r"(function () { function f(x) { return x + 1; } console.log(f?.(1)); })();",
        "2",
    ),
    // call proxy called with `?.`
    (
        r"(function () { function t(a, b) { return a + b; } function p(a, b) { return t(a, b); } console.log(p?.(1, 2)); })();",
        "3",
    ),
    // trampoline called with `?.`
    (
        r#"var bmak = 1, _abck = 2; (function () { var S = 1; function D(s, args) { return "d" + s + args[0]; } var T = function () { return D.apply(this, [S, arguments]); }; console.log(T?.(5), T(6)); })();"#,
        "d15 d16",
    ),
    // lookup forwarder called with `?.`
    (
        r#"var bmak = 1, _abck = 2; (function () { function A() { return ["a", "b", "c"]; } function F(i) { return A()[i]; } console.log(F?.(0), F(2)); })();"#,
        "a c",
    ),
    // method-call forwarder called with `?.`
    (
        r"var bmak = 1, _abck = 2; (function () { function M(o, a) { return o.concat(a); } console.log(M?.([1], 2), M([3], 4)); })();",
        "[ 1, 2 ] [ 3, 4 ]",
    ),
    // trampoline assigned in one `case` and called in another, the BMP async state-machine shape
    (
        r#"var bmak = 1, _abck = 2; (function () { var S = 1; function D(s, args) { return "d" + s + args[0]; } function run(state) { var T; switch (state) { case 0: T = function () { return D.apply(this, [S, arguments]); }; case 1: try { return T(5); } catch (e) { return "TE"; } } } console.log(run(0), run(1)); })();"#,
        "d15 TE",
    ),
];

#[test]
fn inlined_proxies_keep_their_behavior() {
    let dir = std::env::temp_dir().join(format!("jsb-proxy-inlining-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    for flags in [&["-d"][..], &["-d", "--isolated"]] {
        for &(input, expected) in CASES {
            let output = deobfuscate(&dir, input, flags);
            let Some(input_stdout) = node_stdout(&dir, input) else {
                eprintln!("node not found; skipping the behavior comparison");
                return;
            };
            assert_eq!(input_stdout.trim_end(), expected, "{input}");
            assert_eq!(
                node_stdout(&dir, &output).as_deref().map(str::trim_end),
                Some(expected),
                "{flags:?} {input}\n=>\n{output}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
