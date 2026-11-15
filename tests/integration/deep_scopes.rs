//! Input nested so deep that `oxc_semantic` analysis is quadratic: every
//! reference walks the scope chain and every `var` is recorded in each block
//! it is hoisted through. Above a cost budget the pipeline is skipped, so
//! `-d` stays linear and the output must still behave like the input.

use oxc_allocator::Allocator;
use oxc_parser::Parser;
use oxc_span::SourceType;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

fn jsbeautify(args: &[&str], stdin: &str) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_jsbeautify"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn jsbeautify");
    child
        .stdin
        .take()
        .expect("stdin piped")
        .write_all(stdin.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait for jsbeautify");
    assert!(
        output.status.success(),
        "{args:?} failed: {:?}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("UTF-8 output")
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

/// Shapes nested `depth` scopes deep, each printing a count that depends on
/// every level having run.
fn deep_shapes(depth: usize) -> Vec<(&'static str, String)> {
    let levels = |each: &dyn Fn(usize) -> String| (0..depth).map(each).collect::<String>();
    let close = "}".repeat(depth);
    vec![
        (
            "switches declaring functions",
            format!(
                "var n = 0;\n(function (x) {{ {}{close} }})(0);\nconsole.log(n);\n",
                levels(&|i| format!("switch (x) {{ case 0: n++; var w{i} = function (a) {{ return a; }}; w{i}(1); "))
            ),
        ),
        (
            "switches",
            format!(
                "var x = 1, n = 0;\n{}{close}\nconsole.log(n);\n",
                "switch (x) { case 1: n++; ".repeat(depth)
            ),
        ),
        (
            "switches in a loop",
            format!(
                "var n = 0;\nfor (var i = 0; i < 2; i++) {{ {}{close} }}\nconsole.log(n);\n",
                "switch (i) { case 0: case 1: n += i; ".repeat(depth)
            ),
        ),
        (
            "labelled blocks",
            format!(
                "var n = 0;\n{}n++; break L0; n += 5; {close}\nconsole.log(n);\n",
                levels(&|i| format!("L{i}: {{ n++; var v{i} = n; "))
            ),
        ),
    ]
}

/// 3,000 nested switches that each declare a `var` took 8 s and 460 MB;
/// 20,000 levels of any of these shapes took from 6 s to minutes and tens
/// of GB.
#[test]
fn deep_scopes_deobfuscate() {
    for (shape, source) in deep_shapes(20_000) {
        let out = jsbeautify(&["-d", "-"], &source);
        // Parsing only: a semantic analysis of this output is the quadratic
        // cost under test.
        std::thread::scope(|scope| {
            std::thread::Builder::new()
                .stack_size(1 << 30)
                .spawn_scoped(scope, || {
                    let allocator = Allocator::default();
                    let parsed = Parser::new(&allocator, &out, SourceType::cjs()).parse();
                    assert!(parsed.diagnostics.is_empty(), "{shape}: {:?}", parsed.diagnostics);
                })
                .expect("spawn parser thread")
                .join()
                .expect("output parses");
        });
    }
}

/// At 300 levels every shape is deobfuscated; at 1,500, the deepest node
/// runs, the shapes that declare a `var` per level are over the budget and
/// only laid out. Both must print what the input prints.
#[test]
fn deep_scopes_keep_behavior() {
    let dir = std::env::temp_dir().join(format!("jsb-deep-semantic-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    'depths: for depth in [300, 1_500] {
        for (shape, source) in deep_shapes(depth) {
            let Some(expected) = node_stdout(&dir, &source) else {
                eprintln!("node not found; skipping the behavior comparison");
                break 'depths;
            };
            for args in [&["-d", "-"][..], &["-d", "--isolated", "-"]] {
                let out = jsbeautify(args, &source);
                assert_eq!(
                    node_stdout(&dir, &out).as_ref(),
                    Some(&expected),
                    "{shape} {depth} {args:?}"
                );
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
