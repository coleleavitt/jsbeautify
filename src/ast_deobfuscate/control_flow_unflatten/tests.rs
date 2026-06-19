#[cfg(test)]
use super::*;
use crate::ast_deobfuscate::DeobfuscateState;
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

fn run_unflatten(code: &str) -> String {
    let allocator = Allocator::default();
    let source_type = SourceType::mjs();
    let ret = Parser::new(&allocator, code, source_type).parse();
    let mut program = ret.program;

    let mut unflattener = ControlFlowUnflattener::new();
    let state = DeobfuscateState::new();
    let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

    traverse_mut_with_ctx(&mut unflattener, &mut program, &mut ctx);

    Codegen::new().build(&program).code
}

#[test]
fn test_detect_sequence() {
    let code = r#"var _flow = "2|0|1".split('|');"#;
    let allocator = Allocator::default();
    let source_type = SourceType::mjs();
    let ret = Parser::new(&allocator, code, source_type).parse();
    let mut program = ret.program;

    let mut unflattener = ControlFlowUnflattener::new();
    let state = DeobfuscateState::new();
    let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

    traverse_mut_with_ctx(&mut unflattener, &mut program, &mut ctx);

    assert!(unflattener.detected_sequences.contains_key("_flow"));
    assert_eq!(
        unflattener.detected_sequences.get("_flow").unwrap(),
        &vec!["2", "0", "1"]
    );
}

#[test]
fn test_simple_unflatten() {
    let code = r#"
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

    let output = run_unflatten(code);
    eprintln!("Output:\n{output}");

    assert!(output.contains("first"));
    assert!(output.contains("second"));
    assert!(output.contains("third"));
}

#[test]
fn test_no_unflatten_regular_while() {
    let code = r"
var x = 0;
while (x < 10) {
    console.log(x);
    x++;
}
";

    let output = run_unflatten(code);

    assert!(output.contains("while"));
    assert!(output.contains("x < 10"));
}

#[test]
fn test_for_infinite_unflatten() {
    let code = r#"
var _flow = "1|0|2".split('|');
var _i = 0;
for (;;) {
    switch (_flow[_i++]) {
        case "0": console.log("second"); continue;
        case "1": console.log("first"); continue;
        case "2": console.log("third"); break;
    }
    break;
}
"#;

    let output = run_unflatten(code);
    eprintln!("For(;;) Output:\n{output}");

    assert!(output.contains("first"));
    assert!(output.contains("second"));
    assert!(output.contains("third"));
    // Should NOT contain for loop anymore
    assert!(!output.contains("for"));
    assert!(!output.contains("switch"));
}
