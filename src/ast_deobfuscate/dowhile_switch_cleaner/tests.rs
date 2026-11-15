use super::*;
use crate::ast_deobfuscate::semantic;
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_span::SourceType;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

/// `code` with its dead cases pruned, and how many were.
fn run(code: &str) -> (String, usize) {
    let allocator = Allocator::default();
    let mut program = Parser::new(&allocator, code, SourceType::mjs()).parse().program;
    let scoping = semantic::scoping(&program);
    let mut cleaner = DoWhileSwitchCleaner::new(&program, &scoping);
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
    traverse_mut_with_ctx(&mut cleaner, &mut program, &mut ctx);
    (Codegen::new().build(&program).code, cleaner.pruned_cases())
}

/// A dispatcher over the constants `A`–`E` and `EXIT`, with `cases` as its
/// switch body, called as `calls`.
fn dispatcher(cases: &str, calls: &str) -> String {
    format!(
        "var A = 1, B = 2, C = 3, D = 4, E = 5, EXIT = 99;
         var F = function G(s, a) {{
             do {{
                 switch (s) {{ {cases} }}
             }} while (s != EXIT);
         }};
         {calls}"
    )
}

fn kept(out: &str) -> Vec<char> {
    ['A', 'B', 'C', 'D', 'E']
        .into_iter()
        .filter(|label| out.contains(&format!("case {label}:")))
        .collect()
}

#[test]
fn prunes_unreachable_case() {
    let (out, pruned) = run(&dispatcher(
        "case A: { foo(); s = B; } break;
         case B: { bar(); s = EXIT; } break;
         case C: { baz(); s = EXIT; } break;",
        "F(A, []);",
    ));
    assert_eq!(pruned, 1, "{out}");
    assert_eq!(kept(&out), ['A', 'B']);
}

#[test]
fn follows_chains_and_conditional_edges() {
    let (out, pruned) = run(&dispatcher(
        "case A: { s = B; } break;
         case B: { if (a[0]) { s = C; } else { s = D; } } break;
         case C: { s = EXIT; } break;
         case D: { s = EXIT; } break;
         case E: { s = EXIT; } break;",
        "F(A, []);",
    ));
    assert_eq!(pruned, 1, "{out}");
    assert_eq!(kept(&out), ['A', 'B', 'C', 'D']);
}

#[test]
fn counts_pruned_cases() {
    let (_, pruned) = run(&dispatcher(
        "case A: s = EXIT; break; case B: s = EXIT; break; case C: s = EXIT; break; case D: s = EXIT; break;",
        "F(A, []);",
    ));
    assert_eq!(pruned, 3);
}

#[test]
fn entries_through_call_and_the_function_name() {
    let (out, pruned) = run(&dispatcher(
        "case A: s = EXIT; break; case B: G(C, a); s = EXIT; break; case C: s = EXIT; break; case D: s = EXIT; break;",
        "F(A, []); F.call(this, B, []);",
    ));
    assert_eq!(pruned, 1, "{out}");
    assert_eq!(kept(&out), ['A', 'B', 'C']);
}

#[test]
fn a_case_without_a_write_keeps_its_state() {
    // `A` loops until its body leaves; nothing else is reachable.
    let (out, pruned) = run(&dispatcher(
        "case A: if (foo()) return; break; case B: s = EXIT; break;",
        "F(A, []);",
    ));
    assert_eq!(pruned, 1, "{out}");
    assert_eq!(kept(&out), ['A']);
}

#[test]
fn resolves_compound_transitions_by_value() {
    let code = "var X = 10, K = 3, Y = 13, Z = 15, EXIT = 99;
        var F = function (s, a) {
            do {
                switch (s) {
                    case X: { foo(); s += K; } break;
                    case Y: { bar(); s = EXIT; } break;
                    case Z: { s = EXIT; } break;
                }
            } while (s != EXIT);
        };
        function unrelated() { var K = 5; return K; }
        F(X, []);";
    let (out, pruned) = run(code);
    assert_eq!(pruned, 1, "{out}");
    assert!(out.contains("case Y:") && !out.contains("case Z:"), "{out}");
}

#[test]
fn falls_through_into_the_next_case() {
    let (out, pruned) = run(&dispatcher(
        "case A: foo(); s = EXIT; case B: bar(); break; case C: s = EXIT; break;",
        "F(A, []);",
    ));
    assert_eq!(pruned, 1, "{out}");
    assert_eq!(kept(&out), ['A', 'B']);
}

#[test]
fn a_later_case_with_the_same_value_is_never_selected() {
    let code = "var A = 1, B = 2, C = 1, EXIT = 99;
        var F = function (s, a) {
            do { switch (s) { case A: s = C; break; case B: s = EXIT; break; case C: s = EXIT; break; } } while (s != EXIT);
        };
        F(A, []);";
    let (out, pruned) = run(code);
    assert_eq!(pruned, 2, "{out}");
    assert!(out.contains("case A:") && !out.contains("case C:"), "{out}");
}

#[test]
fn keeps_every_case_when_the_state_may_hold_another_value() {
    for (cases, calls) in [
        // The block's `B` is 3.
        (
            "case A: { let B = 3; s = B; } break; case B: break; case C: break;",
            "F(A, []);",
        ),
        (
            "case A: s = B; if (a.length === 0) s = foo(); break; case B: break; case C: break;",
            "F(A, []);",
        ),
        (
            "case A: s = B; bump(); break; case B: break; case C: break; case D: var bump = function () { s = C; }; break;",
            "F(A, []);",
        ),
        (
            "case A: s = B; arguments[0] = C; break; case B: break; case C: break;",
            "F(A, []);",
        ),
        // The assignment after `break` never runs, but `C` stays reachable.
        (
            "case A: s = C; break; s = B; case B: break; case C: break;",
            "F(A, []);",
        ),
        (
            "case A: s = EXIT; break; case B: break; case C: break;",
            "F(A, []); var H = F; H(C, []);",
        ),
        (
            "case A: s = EXIT; break; case B: break; case C: break;",
            "F(A, []); F(3 + foo(), []);",
        ),
        (
            "case A: s = 7; break; case B: break; default: s = C; break; case C: break;",
            "F(A, []);",
        ),
    ] {
        let code = dispatcher(cases, calls);
        let (out, pruned) = run(&code);
        assert_eq!(pruned, 0, "{code}\n{out}");
    }
}

#[test]
fn keeps_every_case_without_a_known_call() {
    let (_, pruned) = run(&dispatcher("case A: s = EXIT; break; case B: s = EXIT; break;", ""));
    assert_eq!(pruned, 0);
}

#[test]
fn prunes_a_switch_that_is_the_loop_body() {
    let code = "var A = 1, B = 2, EXIT = 99;
        var F = function (s, a) { foo(); do switch (s) { case A: s = EXIT; break; case B: s = EXIT; break; } while (s !== EXIT); };
        F(A, []);";
    let (out, pruned) = run(code);
    assert_eq!(pruned, 1, "{out}");
    assert!(out.contains("case A:") && !out.contains("case B:"), "{out}");
}

#[test]
fn keeps_unreachable_cases_that_declare_bindings() {
    for (dead, declares) in [
        ("function h() {}", true),
        ("let x = 1;", true),
        ("const x = 1;", true),
        ("class K {}", true),
        ("var x = 1;", true),
        ("if (a) { for (var i in a) {} }", true),
        ("{ function h() {} }", true),
        ("{ let x = 1; }", false),
        ("var f = null; f = function () { var y; };", true),
        ("(function () { var y; function g() {} })();", false),
        ("(() => { var y; })();", false),
    ] {
        let code = dispatcher(&format!("case A: s = EXIT; break; case B: {dead} break;"), "F(A, []);");
        let (out, pruned) = run(&code);
        assert_eq!(pruned, usize::from(!declares), "{dead}\n{out}");
    }
}
