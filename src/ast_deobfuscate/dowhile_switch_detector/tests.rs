use oxc_allocator::Allocator;
use oxc_parser::Parser;
use oxc_span::SourceType;

use super::*;
use crate::ast_deobfuscate::semantic;

/// The dispatchers of `code`, parsed as a module, by the name of the
/// binding each is assigned to.
fn detect_in(code: &str) -> Vec<(String, DoWhileDispatcher)> {
    let allocator = Allocator::default();
    let program = Parser::new(&allocator, code, SourceType::mjs()).parse().program;
    let scoping = semantic::scoping(&program);
    let mut found: Vec<_> = detect(&program, &scoping)
        .into_iter()
        .map(|(symbol, dispatcher)| (scoping.symbol_name(symbol).to_string(), dispatcher))
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

fn n(value: i64) -> StateValue {
    StateValue::Number(value)
}

fn s(value: &str) -> StateValue {
    StateValue::String(value.into())
}

/// A dispatcher over the constants `A`–`D` and `END`, with `cases` as its
/// switch body and `calls` after it.
fn dispatcher(consts: &str, cases: &str, calls: &str) -> String {
    format!(
        "var A = 1, B = 2, C = 3, D = 4, END = 9{consts};
         var F = function (s, a) {{
             do {{
                 switch (s) {{ {cases} }}
             }} while (s != END);
         }};
         {calls}"
    )
}

fn only(code: &str) -> DoWhileDispatcher {
    let mut found = detect_in(code);
    assert_eq!(found.len(), 1, "{code}");
    let (name, dispatcher) = found.remove(0);
    assert_eq!(name, "F");
    dispatcher
}

fn rejected(code: &str) {
    assert!(detect_in(code).is_empty(), "{code}");
}

#[test]
fn records_labels_transitions_and_entries() {
    let d = only(&dispatcher(
        "",
        "case A: foo(); s = B; break;
         case B: if (a[0]) { s = C; } else { s = END; } break;
         case C: return 1;",
        "F(A, []); F.call(null, C, []);",
    ));
    let labels: Vec<_> = d.cases.iter().map(|case| case.label.clone()).collect();
    assert_eq!(labels, [n(1), n(2), n(3)]);
    assert_eq!(d.cases[0].next, [n(2)]);
    assert_eq!(d.cases[1].next, [n(3), n(9)]);
    assert_eq!(d.cases[2].next, Vec::<StateValue>::new());
    assert!(d.cases.iter().all(|case| !case.falls_through));
    assert_eq!(d.entries, [n(1), n(3)]);
    assert_eq!((d.loop_index, d.switch_index), (0, Some(0)));
}

#[test]
fn finds_the_switch_in_a_loop_block() {
    let code = "var A = 1, END = 9;
        var F = function (s, a) { var n = 0; do { n++; switch (s) { case A: s = END; } } while (s !== END); };
        F(A, 0);";
    let d = only(code);
    assert_eq!((d.loop_index, d.switch_index), (1, Some(1)));
    assert!(d.cases[0].falls_through);
}

#[test]
fn computes_values_as_javascript_does() {
    let d = only(&dispatcher(
        ", X = [1] + [2], Y = +([1] + [2]), Z = [1] + 2, W = [] + [], V = [6] * [7] - 2",
        "case X: case Y: case Z: case W: case V: break;",
        "",
    ));
    let labels: Vec<_> = d.cases.into_iter().map(|case| case.label).collect();
    assert_eq!(labels, [s("12"), n(12), s("12"), s(""), n(40)]);
}

#[test]
fn resolves_compound_transitions() {
    let d = only(&dispatcher(
        ", K = 3",
        "case A: s += K; break; case B: s -= K; break;",
        "",
    ));
    assert_eq!(d.cases[0].next, [n(4)]);
    assert_eq!(d.cases[1].next, [n(-1)]);
}

#[test]
fn reads_constants_through_their_bindings() {
    // The case body's `B` is the block's `let`, not the constant.
    let d = only(&dispatcher(
        ", Q = C",
        "case A: { let B = 3; s = Q; } break; case B: break;",
        "",
    ));
    assert_eq!(d.cases[0].next, [n(3)]);
    rejected(&dispatcher("", "case A: { let B = 3; s = B; } break;", ""));
}

#[test]
fn rejects_unknown_writes_to_the_state() {
    for case in [
        "case A: s = B; if (a) s = foo(); break;",
        "case A: s++; break;",
        "case A: [s] = [B]; break;",
        "case A: s = B; var g = function () { s = C; }; break;",
        "case A: s = B; (() => { s = C; })(); break;",
        "case A: s = B; arguments[0] = C; break;",
        "case A: s += B; s += B; break;",
        "case A: while (a()) s += B; break;",
        "case A: s = C; s += B; break;",
        "case A: s = B; case B: s += C; break;",
        "case A: s = B; break; default: s = C;",
        "case foo(): s = B; break;",
    ] {
        rejected(&dispatcher("", case, "F(A, []);"));
    }
    let outside = "var A = 1, B = 2, END = 9;
        var F = function (s, a) { s = B; do { switch (s) { case A: s = END; } } while (s != END); };";
    rejected(outside);
}

#[test]
fn rejects_calls_it_cannot_follow() {
    for calls in [
        "F(A, []); var G = F; G(C, []);",
        "F(A, []); F(foo(), []);",
        "F(...[A]);",
        "new F(A, []);",
        "F = null;",
        "export { F };",
    ] {
        rejected(&dispatcher("", "case A: s = END; break;", calls));
    }
}

#[test]
fn constants_must_be_initialised_before_the_dispatcher() {
    // `B` is declared after `F`, so a call can run before it holds 2.
    let code = "var A = 1, END = 9;
        var F = function (s, a) { do { switch (s) { case A: s = B; break; case B: break; } } while (s != END); };
        var B = 2;
        F(A, []);";
    rejected(code);
    // A binding declared in another statement list may be read before its
    // declaration runs.
    let code = "var END = 9;
        function g() { return F(A, []); }
        var F = function (s, a) { do { switch (s) { case END: break; } } while (s != END); };
        g();
        var A = 9;";
    rejected(code);
}

#[test]
fn rejects_redirectable_and_shared_bindings() {
    let in_script = |code: &str| {
        let allocator = Allocator::default();
        let program = Parser::new(&allocator, code, SourceType::script()).parse().program;
        let scoping = semantic::scoping(&program);
        detect(&program, &scoping).len()
    };
    let wrapped = |body: &str| format!("(function () {{ {body} }})();");
    let plain = dispatcher("", "case A: s = END; break;", "F(A, []);");
    assert_eq!(in_script(&wrapped(&plain)), 1);
    // `with` can read another `A`.
    let with = dispatcher("", "case A: s = END; break;", "F(A, []); with ({}) { F(A, []); }");
    assert_eq!(in_script(&wrapped(&with)), 0);
    // An Annex B block function overwrites `A` when its block runs.
    assert_eq!(in_script(&wrapped(&format!("{plain} {{ function A() {{}} }}"))), 0);
    // Other scripts can call a classic script's top-level function and
    // change its top-level `var`s.
    assert_eq!(in_script(&plain), 0);
}

#[test]
fn direct_eval_disables_the_analysis() {
    rejected(&dispatcher(
        "",
        "case A: s = END; break;",
        "F(A, []); function h() { eval('A = 2'); }",
    ));
}

#[test]
fn bounds_hostile_values() {
    // Doubling strings stops at the length cap; integers past 2^53 are not
    // evaluated, because JavaScript rounds them.
    let mut consts = String::from(", S0 = [1] + [1]");
    consts.extend((1..80).map(|i| format!(", S{i} = S{} + S{}", i - 1, i - 1)));
    consts.push_str(", L = 9007199254740992 + 2, M = 9007199254740994");
    rejected(&dispatcher(&consts, "case S79: s = END; break;", ""));
    rejected(&dispatcher(&consts, "case L: s = END; break;", ""));
    rejected(&dispatcher(&consts, "case M: s = END; break;", ""));
}
