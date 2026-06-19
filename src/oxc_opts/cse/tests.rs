use super::CommonSubexpressionElimination;
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_span::SourceType;

fn test_cse(input: &str, should_change: bool) {
    let allocator = Allocator::default();
    let source_type = SourceType::mjs();
    let ret = Parser::new(&allocator, input, source_type).parse();
    let mut program = ret.program;

    let mut cse = CommonSubexpressionElimination::new();
    let changed = cse.run(&mut program, &allocator);

    assert_eq!(changed, should_change);

    if changed {
        let output = Codegen::new().build(&program).code;
        println!("CSE output: {output}");
    }
}

#[test]
fn test_cse_simple() {
    test_cse("{ const a = b + c; const d = b + c; }", true);
}

#[test]
fn test_cse_no_common() {
    test_cse("{ const a = b + c; const d = e + f; }", false);
}

#[test]
fn test_cse_complex_expression() {
    test_cse("{ const a = expensive(x, y); const b = expensive(x, y); }", true);
}
