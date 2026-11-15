//! Inlining passes replace a binding's references with what its declaration
//! holds, and often delete the declaration. That is only sound when every
//! reference is a call that reaches the declaration's value:
//!
//! - a direct `eval` can call, reassign or shadow a binding it can see by name
//!   (`with_scope::redirectable_symbols`);
//! - a `var` or `let` holds nothing until its initializer runs
//!   (`call_sites::initialized_early`);
//! - an optional call `f?.()` is not rewritten, so it still needs `f`.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

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
    // a proxy read through a function an earlier assignment names
    (
        r#"(function () { var x; x = function () { return f(1, 2); }; try { console.log(x()); } catch (e) { console.log("threw"); } var f = function (a, b) { return a + b; }; console.log(f(3, 4)); })();"#,
        "threw\n7",
    ),
    // a proxy read through a parameter reached by `arguments`
    (
        r#"(function () { function o(p) { p = function () { return f(1, 2); }; try { console.log(arguments[0]()); } catch (e) { console.log("threw"); } var f = function (a, b) { return a + b; }; return f(3, 4); } console.log(o(0)); })();"#,
        "threw\n7",
    ),
    // a proxy read through a function declaration hoisted out of a later case
    (
        r"(function () { function run(n) { switch (n) { case 0: try { console.log(g()); } catch (e) { console.log(e.name); } case 1: var f = function (a, b) { return a + b; }; function g() { return f(1, 2); } console.log(f(3, 4)); } } run(0); })();",
        "TypeError\n7",
    ),
    // a proxy read by a case test before the case that sets it
    (
        r"(function () { function run(n) { try { switch (n) { case g(): break; case 1: var f = function (a, b) { return a + b; }; function g() { return f(1, 2); } console.log(f(3, 4)); } } catch (e) { console.log(e.name); } } run(1); })();",
        "TypeError",
    ),
    // a proxy read later through an outer binding, after an early `return`
    (
        r"(function () { var x; function F(c) { x = function () { return f(1, 2); }; if (c) return; var f = function (a, b) { return a + b; }; return f(3, 4); } F(true); try { console.log(x()); } catch (e) { console.log(e.name); } console.log(F(false)); })();",
        "TypeError\n7",
    ),
    // a proxy set and called in the same case is still inlined
    (
        r#"(function () { function run(n) { var r; switch (n) { case 0: var g = function () { return f(1, 2); }; var f = function (a, b) { return a + b; }; r = g(); break; case 1: r = "one"; } return r; } console.log(run(0), run(1)); })();"#,
        "3 one",
    ),
    // a strict switch: a later case calls a function hoisted from an earlier one
    (
        r#"(function () { "use strict"; function run(s) { try { switch (s) { case 1: var f = function (a, b) { return a + b; }; function g() { return f(1, 2); } return f(3, 4); case 0: return g(); } } catch (e) { return e.name; } } console.log(run(0), run(1)); })();"#,
        "TypeError 7",
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

/// What `code` prints run by node as a classic script, whose top-level
/// bindings are properties of the global object.
fn node_script_stdout(code: &str) -> Option<String> {
    let mut child = match Command::new("node")
        .args([
            "-e",
            "require('vm').runInThisContext(require('fs').readFileSync(0, 'utf8'))",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => panic!("run node: {error}"),
    };
    child
        .stdin
        .take()
        .expect("node stdin")
        .write_all(code.as_bytes())
        .expect("write to node");
    let output = child.wait_with_output().expect("run node");
    assert!(output.status.success(), "node failed on:\n{code}");
    Some(String::from_utf8(output.stdout).expect("stdout is utf8"))
}

#[test]
fn proxies_reachable_through_script_globals_keep_their_behavior() {
    // `globalThis.x()` calls the global `x` without naming it, while `f` is
    // still in its temporal dead zone.
    let input = "{ var x = function () { return f(1, 2); }; try { console.log(globalThis.x()); } \
                 catch (e) { console.log(e.name); } let f = function (a, b) { return a + b; }; console.log(f(3, 4)); }";
    let dir = std::env::temp_dir().join(format!("jsb-proxy-script-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let output = deobfuscate(&dir, input, &["-d"]);
    let _ = std::fs::remove_dir_all(&dir);
    let Some(input_stdout) = node_script_stdout(input) else {
        eprintln!("node not found; skipping the behavior comparison");
        return;
    };
    assert_eq!(input_stdout.trim_end(), "ReferenceError\n7");
    assert_eq!(node_script_stdout(&output), Some(input_stdout), "{output}");
}
