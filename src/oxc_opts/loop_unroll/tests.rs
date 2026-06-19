use super::LoopUnroller;
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_span::SourceType;

fn test_unroll(input: &str, expected_contains: &[&str]) {
    let allocator = Allocator::default();
    let source_type = SourceType::mjs();
    let ret = Parser::new(&allocator, input, source_type).parse();
    let mut program = ret.program;

    let mut unroller = LoopUnroller::new();
    let _changed = unroller.run(&mut program, &allocator);

    let output = Codegen::new().build(&program).code;

    for expected in expected_contains {
        assert!(
            output.contains(expected),
            "Expected output to contain '{expected}', got:\n{output}"
        );
    }
}

#[test]
fn test_unroll_simple_loop() {
    test_unroll(
        "for (let i = 0; i < 3; i++) { console.log(i); }",
        &["console.log(0)", "console.log(1)", "console.log(2)"],
    );
}

#[test]
fn test_no_unroll_large_loop() {
    let allocator = Allocator::default();
    let source_type = SourceType::mjs();
    let input = "for (let i = 0; i < 100; i++) { console.log(i); }";
    let ret = Parser::new(&allocator, input, source_type).parse();
    let mut program = ret.program;

    let mut unroller = LoopUnroller::new();
    let changed = unroller.run(&mut program, &allocator);

    assert!(!changed, "Should not unroll loop with 100 iterations");
}
