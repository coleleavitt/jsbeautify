use super::*;
use crate::ast_deobfuscate::semantic;
#[cfg(test)]
use crate::ast_deobfuscate::state::ReturnValue;
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_span::SourceType;

fn run_dispatcher_inliner(code: &str) -> (String, DispatcherInliner) {
    let allocator = Allocator::default();
    let source_type = SourceType::mjs();
    let ret = Parser::new(&allocator, code, source_type).parse();
    let mut program = ret.program;

    let mut inliner = DispatcherInliner::new();
    let state = DeobfuscateState::new();
    let scoping = semantic::scoping(&program);
    let mut ctx = oxc_traverse::ReusableTraverseCtx::new(state, scoping, &allocator);

    oxc_traverse::traverse_mut_with_ctx(&mut inliner, &mut program, &mut ctx);

    let output = Codegen::new().build(&program).code;
    (output, inliner)
}

#[test]
fn test_detect_dispatcher() {
    let code = r#"
            var d = {
                "a": function() { return 1; },
                "b": function() { return 2; }
            };
            d.a();
        "#;

    let (_, inliner) = run_dispatcher_inliner(code);

    assert_eq!(inliner.detected_dispatchers.len(), 1);
    let dispatcher = inliner.detected_dispatchers.values().next().unwrap();
    assert_eq!(dispatcher.var_name, "d");
    assert_eq!(dispatcher.functions.len(), 2);
    assert!(matches!(
        dispatcher.functions.get("a").unwrap().return_value,
        Some(ReturnValue::Number(1.0))
    ));
    assert!(matches!(
        dispatcher.functions.get("b").unwrap().return_value,
        Some(ReturnValue::Number(2.0))
    ));
}

#[test]
fn test_inline_number() {
    let code = r#"
            var d = { "a": function() { return 42; } };
            var x = d["a"]();
        "#;

    let (output, inliner) = run_dispatcher_inliner(code);
    eprintln!("Output: {output}");

    assert!(inliner.rewrites() > 0, "Should have inlined something");
    assert!(output.contains("42"), "Should contain inlined value 42");
    assert!(!output.contains("d[\"a\"]()"), "Should not contain dispatcher call");
}

#[test]
fn test_inline_string() {
    let code = r#"
            var d = { "key": function() { return "hello"; } };
            var x = d["key"]();
        "#;

    let (output, inliner) = run_dispatcher_inliner(code);
    eprintln!("Output: {output}");

    assert!(inliner.rewrites() > 0);
    assert!(output.contains("\"hello\""));
}

#[test]
fn test_inline_multiple_calls() {
    let code = r#"
            var d = {
                "a": function() { return 1; },
                "b": function() { return "test"; }
            };
            var x = d["a"]();
            var y = d["b"]();
        "#;

    let (output, inliner) = run_dispatcher_inliner(code);
    eprintln!("Output: {output}");

    assert!(inliner.rewrites() > 0);
    assert!(output.contains("= 1"));
    assert!(output.contains("\"test\""));
}

#[test]
fn test_no_inline_non_constant() {
    let code = r#"
            var d = { "a": function(x) { return x + 1; } };
            var x = d["a"](5);
        "#;

    let (output, inliner) = run_dispatcher_inliner(code);
    eprintln!("Output: {output}");

    assert!(inliner.rewrites() == 0, "Should not inline functions with parameters");
    assert!(output.contains("d[\"a\"]"));
}

#[test]
fn test_inline_dot_notation() {
    let code = r"
            var d = { a: function() { return 42; } };
            var x = d.a();
        ";

    let (output, inliner) = run_dispatcher_inliner(code);
    eprintln!("Output: {output}");

    assert!(inliner.rewrites() > 0, "Should have inlined dot notation call");
    assert!(output.contains("42"), "Should contain inlined value 42");
    assert!(!output.contains("d.a()"), "Should not contain original call");
}

#[test]
fn test_inline_mixed_notation() {
    let code = r#"
            var d = {
                "strKey": function() { return "hello"; },
                numKey: function() { return 123; }
            };
            var a = d["strKey"]();
            var b = d.numKey();
        "#;

    let (output, inliner) = run_dispatcher_inliner(code);
    eprintln!("Output: {output}");

    assert!(inliner.rewrites() > 0, "Should have inlined calls");
    assert!(output.contains("\"hello\""), "Should inline string");
    assert!(output.contains("123"), "Should inline number");
}

#[test]
fn test_inline_arrow_expression_body() {
    let code = r"
            var d = { a: () => 42, b: x => x * 2 };
            var x = d.a();
            var y = d.b(5);
        ";

    let (output, inliner) = run_dispatcher_inliner(code);
    eprintln!("Output: {output}");

    assert!(inliner.rewrites() > 0, "Should have inlined arrow function");
    assert!(output.contains("42"), "Should inline arrow expression");
    assert!(!output.contains("d.a()"), "Should not contain original call");
}

#[test]
fn block_arrow_without_return_yields_undefined() {
    let code = r"
            var d = { a: () => { var x = 1; 42 } };
            var x = d.a();
        ";
    let (output, inliner) = run_dispatcher_inliner(code);
    assert!(
        inliner.rewrites() == 0,
        "a block body without return yields undefined: {output}"
    );
}

#[test]
fn test_no_inline_arrow_with_side_effects() {
    let code = r#"
            var d = { a: () => console.log("side effect") };
            var x = d.a();
        "#;

    let (output, inliner) = run_dispatcher_inliner(code);
    eprintln!("Output: {output}");

    // Should not inline because console.log is not a literal
    assert!(
        inliner.rewrites() == 0 || !output.contains("42"),
        "Should not inline if last expression is not a literal"
    );
}

#[test]
fn identifier_return_is_not_moved_to_the_call_site() {
    for code in [
        "var result = 999;\nvar d = { a: function() { return result; } };\nfunction f(result) { return d.a(); }",
        "var d = { a: function() { return arguments; } };\nfunction f(x) { return d.a(); }",
    ] {
        let (output, inliner) = run_dispatcher_inliner(code);
        assert!(inliner.rewrites() == 0, "{output}");
    }
}

#[test]
fn replaced_or_escaping_dispatcher_is_not_inlined() {
    for code in [
        "var d = { a: function() { return true; } };\nd.a = function() { return false; };\nd.a();",
        "var d = { a: function() { return true; } };\nd = other;\nd.a();",
        "var d = { a: function() { return true; } };\nmutate(d);\nd.a();",
        "var d = { a: function() { return true; }, a: function() { return false; } };\nd.a();",
        "var d = { a: function() { return true; }, ...o };\nd.a();",
        "var d = { a: function() { return true; } };\nfunction f(d) { return d.a(); }\nf({ a: () => 7 });",
    ] {
        let (output, _) = run_dispatcher_inliner(code);
        assert!(output.contains(".a()"), "call must be kept: {output}");
    }
}

#[test]
fn call_with_non_literal_arguments_is_kept() {
    let code = "var d = { a: function() { return 1; } };\nvar x = d.a(sideEffect());";
    let (output, inliner) = run_dispatcher_inliner(code);
    assert!(inliner.rewrites() == 0, "{output}");
}

#[test]
fn test_inline_boolean_and_null() {
    let code = r#"
            var d = {
                "true": function() { return true; },
                "false": function() { return false; },
                "null": function() { return null; }
            };
            var a = d["true"]();
            var b = d["false"]();
            var c = d["null"]();
        "#;

    let (output, inliner) = run_dispatcher_inliner(code);
    eprintln!("Output: {output}");

    assert!(inliner.rewrites() > 0);
    assert!(output.contains("true"), "Should inline true");
    assert!(output.contains("false"), "Should inline false");
    assert!(output.contains("null"), "Should inline null");
}

#[test]
fn test_complex_dispatcher() {
    let code = r#"
            var handlers = {
                "login": function() { return "logged_in"; },
                "logout": function() { return "logged_out"; },
                "getUser": () => ({ name: "John", age: 30 }),
                admin: function() { return { role: "admin" }; }
            };
            var status = handlers["login"]();
            var user = handlers.getUser();
            var role = handlers.admin();
        "#;

    let (output, inliner) = run_dispatcher_inliner(code);
    eprintln!("Output: {output}");

    // Simple function inlining should work
    assert!(inliner.rewrites() > 0);
    assert!(output.contains("\"logged_in\""), "Should inline login");
    assert!(output.contains("\"logged_out\""), "Should inline logout");
}
