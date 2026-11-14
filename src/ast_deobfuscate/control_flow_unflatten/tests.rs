use super::*;
use crate::ast_deobfuscate::{DeobfuscateState, semantic};
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_span::SourceType;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

fn run_with(code: &str, source_type: SourceType) -> (String, usize) {
    let allocator = Allocator::default();
    let mut program = Parser::new(&allocator, code, source_type).parse().program;
    let mut unflattener = ControlFlowUnflattener::new();
    let scoping = semantic::scoping(&program);
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
    traverse_mut_with_ctx(&mut unflattener, &mut program, &mut ctx);
    (Codegen::new().build(&program).code, unflattener.rewrites())
}

fn run(code: &str) -> (String, usize) {
    run_with(code, SourceType::mjs())
}

fn assert_kept(code: &str) {
    let (out, rewrites) = run(code);
    assert_eq!(rewrites, 0, "{out}");
    assert!(out.contains("switch"), "{out}");
}

const LOOP: &str = r#"
var _flow = "2|0|1".split('|');
var _i = 0;
while (true) {
    switch (_flow[_i++]) {
        case "0": console.log("second"); continue;
        case "1": console.log("third"); break;
        case "2": console.log("first"); continue;
    }
    break;
}
"#;

#[test]
fn unflattens_in_sequence_order() {
    let (out, rewrites) = run(LOOP);
    assert_eq!(rewrites, 1, "{out}");
    assert_eq!(
        out,
        "console.log(\"first\");\nconsole.log(\"second\");\nconsole.log(\"third\");\n"
    );
}

#[test]
fn unflattens_obfuscator_io_loop_with_not_not_array_test() {
    let code = r#"var a=[];var o="1|0".split("|");var i=0;while(!![]){switch(o[i++]){case"0":a.push("x");continue;case"1":a.push("y");continue;}break;}console.log(a.join(","));"#;
    let (out, rewrites) = run(code);
    assert_eq!(rewrites, 1, "{out}");
    assert_eq!(
        out,
        "var a = [];\na.push(\"y\");\na.push(\"x\");\nconsole.log(a.join(\",\"));\n"
    );
}

#[test]
fn unflattens_inside_a_function_body_even_in_a_script() {
    let code = r#"function f(p){var a=[];var o="1|0".split("|"),i=0;while(true){switch(o[i++]){case"0":a.push(p);continue;case"1":a.push("y");continue;}break;}return a;}"#;
    let (out, rewrites) = run_with(code, SourceType::script());
    assert_eq!(rewrites, 1, "{out}");
    assert!(!out.contains("split") && !out.contains("while"), "{out}");
    assert!(out.contains("a.push(\"y\");\n\ta.push(p);"), "{out}");
}

#[test]
fn unflattens_for_infinite_loop() {
    let (out, rewrites) = run(&LOOP.replace("while (true)", "for (;;)"));
    assert_eq!(rewrites, 1, "{out}");
    assert!(
        !out.contains("for") && !out.contains("switch") && !out.contains("_flow"),
        "{out}"
    );
}

#[test]
fn keeps_script_top_level_globals() {
    let (out, rewrites) = run_with(LOOP, SourceType::script());
    assert_eq!(rewrites, 0, "{out}");
    assert!(out.contains("_flow") && out.contains("_i"), "{out}");
}

#[test]
fn keeps_declarations_when_the_loop_is_not_a_dispatcher() {
    for code in [
        r#"var _flow = "2|0|1".split('|'); var _i = 0; f(_flow, _i);"#,
        r#"var _flow = "1|0".split('|'); var _i = 0; while (g()) { switch (_flow[_i++]) { case "0": continue; } break; }"#,
    ] {
        let (out, rewrites) = run(code);
        assert_eq!(rewrites, 0, "{out}");
        assert!(out.contains("_flow") && out.contains("_i"), "{out}");
    }
}

#[test]
fn keeps_loops_whose_state_is_read_elsewhere() {
    assert_kept(&format!("{LOOP}\nconsole.log(_i);"));
    assert_kept(&format!("{LOOP}\nconsole.log(_flow);"));
    assert_kept(&LOOP.replace("console.log(\"second\")", "eval(\"_i\")"));
    assert_kept(&LOOP.replace("var _i = 0;", "var _i = 1;"));
    assert_kept(&LOOP.replace("var _i = 0;", "const _i = 0;"));
    assert_kept(
        &LOOP
            .replace("var _i = 0;\n", "")
            .replace("break;\n}\n", "break;\n}\nvar _i = 0;\n"),
    );
}

#[test]
fn keeps_loops_with_extra_statements_or_unsupported_cases() {
    assert_kept(&LOOP.replace("    break;\n}", "    g();\n    break;\n}"));
    assert_kept(&LOOP.replace("case \"0\"", "default"));
    assert_kept(&LOOP.replace("case \"0\"", "case 0"));
    assert_kept(&LOOP.replace("console.log(\"second\");", "let s = 1;"));
    assert_kept(&LOOP.replace("console.log(\"second\");", "if (g()) continue;"));
    assert_kept(&LOOP.replace("console.log(\"second\");", "if (g()) break;"));
    assert_kept(&LOOP.replace("console.log(\"second\"); continue;", "console.log(\"second\");"));
}

#[test]
fn stops_at_a_breaking_case_or_an_unknown_step() {
    let (out, _) = run(&LOOP.replace("\"2|0|1\"", "\"2|1|0\""));
    assert!(!out.contains("second"), "{out}");
    let (out, _) = run(&LOOP.replace("\"2|0|1\"", "\"2|9|0\""));
    assert_eq!(out, "console.log(\"first\");\n");
}

#[test]
fn keeps_nested_jumps_that_target_inner_statements() {
    let code = LOOP.replace(
        "console.log(\"second\");",
        "for (;;) { if (g()) break; continue; } switch (h) { case 1: break; }",
    );
    let (out, rewrites) = run(&code);
    assert_eq!(rewrites, 1, "{out}");
}
