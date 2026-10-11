use serde_json::Value;
use std::io::Write;
use std::process::{Command, Output, Stdio};

fn jsbeautify(args: &[&str], stdin: &str) -> Output {
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
    child.wait_with_output().expect("wait for jsbeautify")
}

fn json(args: &[&str], stdin: &str) -> Value {
    let output = jsbeautify(args, stdin);
    assert!(output.status.success(), "{args:?} failed: {output:?}");
    serde_json::from_slice(&output.stdout).expect("stdout is JSON")
}

fn positions(references: &Value) -> Vec<(u64, u64, &str)> {
    references
        .as_array()
        .expect("references array")
        .iter()
        .map(|reference| {
            (
                reference["line"].as_u64().expect("line"),
                reference["column"].as_u64().expect("column"),
                reference["access"].as_str().expect("access"),
            )
        })
        .collect()
}

#[test]
fn ast_prints_estree_program() {
    let program = json(&["ast", "-"], "let a = 1;");
    assert_eq!(program["type"], "Program");
    assert_eq!(program["body"][0]["type"], "VariableDeclaration");
    assert_eq!(program["body"][0]["declarations"][0]["init"]["value"], 1);
}

#[test]
fn ast_ts_includes_type_annotations() {
    let program = json(&["ast", "--ts", "-"], "let a: number = 1;");
    let id = &program["body"][0]["declarations"][0]["id"];
    assert_eq!(id["typeAnnotation"]["typeAnnotation"]["type"], "TSNumberKeyword");
}

#[test]
fn ast_fails_on_unrecoverable_parse_error() {
    let output = jsbeautify(&["ast", "-"], "let a = (1;");
    assert!(!output.status.success());
    assert_eq!(output.stdout, b"");
    let stderr = String::from_utf8(output.stderr).expect("stderr is utf8");
    assert!(stderr.contains("let a = (1;"), "diagnostic shows the source: {stderr}");
}

#[test]
fn scope_resolves_shadowed_bindings_to_their_own_symbols() {
    let source = "\
var x = 1;
function f(x) {
  { const x = 2; use(x); }
  x = x + 1;
  return function () { var x; return x; };
}
f(x);
";
    let report = json(&["scope", "-"], source);
    let symbols = report["symbols"].as_array().expect("symbols");
    let xs: Vec<_> = symbols.iter().filter(|symbol| symbol["name"] == "x").collect();
    assert_eq!(xs.len(), 4);

    let summary: Vec<_> = xs
        .iter()
        .map(|symbol| {
            (
                symbol["kind"].as_str().expect("kind"),
                symbol["line"].as_u64().expect("line"),
                positions(&symbol["references"]),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("var", 1, vec![(7, 3, "read")]),
            ("param", 2, vec![(4, 3, "write"), (4, 7, "read")]),
            ("const", 3, vec![(3, 22, "read")]),
            ("var", 5, vec![(5, 38, "read")]),
        ]
    );

    let scopes = report["scopes"].as_array().expect("scopes");
    let block_scope = &scopes[xs[2]["scope"].as_u64().expect("scope") as usize];
    assert_eq!(block_scope["kind"], "BlockStatement");
    assert_eq!(xs[2]["scope_kind"], "BlockStatement");
    assert_eq!(xs[1]["scope_kind"], "Function");

    let unresolved = report["unresolved"].as_array().expect("unresolved");
    assert_eq!(unresolved.len(), 1);
    assert_eq!(unresolved[0]["name"], "use");
    assert_eq!(unresolved[0]["count"], 1);
}

#[test]
fn scope_counts_columns_in_characters_and_reports_all_global_uses() {
    let report = json(&["scope", "-"], "const s = \"\u{e9}\u{1f600}\"; g(s); g = s;");
    let s = &report["symbols"][0];
    assert_eq!(positions(&s["references"]), [(1, 19, "read"), (1, 27, "read")]);
    let g = &report["unresolved"][0];
    assert_eq!(g["count"], 2);
    assert_eq!(positions(&g["references"]), [(1, 17, "read"), (1, 23, "write")]);
}

#[test]
fn scope_reports_recoverable_errors_without_failing() {
    let output = jsbeautify(&["scope", "-"], "let a; let a;");
    assert!(output.status.success(), "{output:?}");
    let stderr = String::from_utf8(output.stderr).expect("stderr is utf8");
    assert!(stderr.contains("already been declared"), "stderr: {stderr}");
}

const LOOP: &str = "\
function sum(xs) {
  let t = 0;
  for (const x of xs) {
    if (x < 0) continue;
    t += x;
  }
  function nested() { while (t) { t--; } }
  return t;
}
const other = () => { if (a) b(); };
";

fn cfg_json(args: &[&str]) -> (Vec<Value>, Vec<Value>) {
    let graph = json(&[&["cfg", "--format", "json", "-"][..], args].concat(), LOOP);
    (
        graph["blocks"].as_array().expect("blocks").clone(),
        graph["edges"].as_array().expect("edges").clone(),
    )
}

fn labels(blocks: &[Value]) -> Vec<String> {
    blocks
        .iter()
        .flat_map(|block| block["instructions"].as_array().expect("instructions"))
        .map(|instruction| instruction["label"].as_str().expect("label").to_owned())
        .collect()
}

#[test]
fn cfg_function_selects_only_that_function_without_nested_functions() {
    let (blocks, edges) = cfg_json(&["--function", "sum"]);
    let labels = labels(&blocks);
    assert!(labels.iter().any(|label| label.starts_with("Iteration(")), "{labels:?}");
    assert!(labels.iter().any(|label| label == "continue"), "{labels:?}");
    assert!(labels.iter().any(|label| label == "return <value>"), "{labels:?}");
    assert!(
        !labels.iter().any(|label| label == "WhileStatement"),
        "nested function leaked: {labels:?}"
    );

    let ids: Vec<_> = blocks.iter().map(|block| block["id"].as_u64().expect("id")).collect();
    assert!(edges.iter().all(|edge| {
        ids.contains(&edge["from"].as_u64().expect("from")) && ids.contains(&edge["to"].as_u64().expect("to"))
    }));
    assert!(edges.iter().any(|edge| edge["kind"] == "Backedge"));
    assert!(edges.iter().all(|edge| edge["kind"] != "NewFunction"));
}

#[test]
fn cfg_function_matches_names_bound_by_declarators() {
    let (blocks, _) = cfg_json(&["--function", "other"]);
    let labels = labels(&blocks);
    assert!(labels.iter().any(|label| label == "IfStatement"), "{labels:?}");
    assert!(!labels.iter().any(|label| label == "continue"), "{labels:?}");
}

#[test]
fn cfg_whole_program_includes_every_function() {
    let (blocks, edges) = cfg_json(&[]);
    let labels = labels(&blocks);
    for expected in ["continue", "WhileStatement", "IfStatement"] {
        assert!(
            labels.iter().any(|label| label == expected),
            "missing {expected}: {labels:?}"
        );
    }
    assert!(edges.iter().any(|edge| edge["kind"] == "NewFunction"));
}

#[test]
fn cfg_prints_dot_by_default() {
    let output = jsbeautify(&["cfg", "--function", "sum", "-"], LOOP);
    assert!(output.status.success(), "{output:?}");
    let dot = String::from_utf8(output.stdout).expect("dot is utf8");
    assert!(dot.starts_with("digraph cfg {"), "{dot}");
    assert!(dot.contains("continue @4:16"), "{dot}");
    assert!(dot.contains("[label=\"Backedge\"]"), "{dot}");
}

#[test]
fn cfg_unknown_function_fails() {
    let output = jsbeautify(&["cfg", "--function", "missing", "-"], LOOP);
    assert!(!output.status.success());
    assert_eq!(output.stdout, b"");
}
