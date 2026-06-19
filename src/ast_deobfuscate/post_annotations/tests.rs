#[cfg(test)]
use super::*;

#[test]
fn annotates_enum_iife() {
    let code = r#"(function(H) { H.HEADER = "header"; H.QUERY = "query"; })(lM.HttpAuthLocation || (lM.HttpAuthLocation = {}));"#;
    let (out, count) = annotate_ts_enums(code);
    assert_eq!(count, 1);
    assert!(out.contains("/* enum HttpAuthLocation"), "Should annotate: {out}");
    assert!(out.contains("HEADER=\"header\""), "Should show members: {out}");
}

#[test]
fn annotates_namespace_module() {
    let code = r"__export(GZ8, { writeToStdout: () => UK, exitWithError: () => Ba4 })";
    let (out, count) = annotate_namespace_modules(code);
    assert_eq!(count, 1);
    assert!(out.contains("/* module GZ8:"), "Should annotate target: {out}");
    assert!(out.contains("writeToStdout"), "Should show export: {out}");
}

#[test]
fn expands_arrow_sequence() {
    let code = "const f = (H) => (validate(H), transform(H), emit(H));";
    let (out, count) = expand_arrow_sequences(code);
    assert_eq!(count, 1);
    assert!(out.contains("validate(H);"), "Should split: {out}");
    assert!(out.contains("return emit(H);"), "Should have return: {out}");
}

#[test]
fn expands_arrow_sequence_with_string_commas_safely() {
    let code = r#"const f = () => ("a,b", work(), done());"#;
    let (out, count) = expand_arrow_sequences(code);
    assert_eq!(count, 1);
    assert!(out.contains(r#""a,b";"#), "string literal must stay intact: {out}");
    assert!(out.contains("work();"), "Should split second part: {out}");
    assert!(out.contains("return done();"), "Should return final part: {out}");
}

#[test]
fn ignores_arrow_text_inside_string() {
    let code = r#"const s = "=> (a(), b(), c())";"#;
    let (out, count) = expand_arrow_sequences(code);
    assert_eq!(count, 0);
    assert_eq!(out, code);
}

#[test]
fn ignores_expression_continuation_after_parenthesized_sequence() {
    let code = "const f = () => (a(), b(), c()) + d;";
    let (out, count) = expand_arrow_sequences(code);
    assert_eq!(count, 0);
    assert_eq!(out, code);
}

#[test]
fn ignores_arrow_text_inside_regex_literal() {
    let code = r"const r = /=> (a(), b(), c())/;";
    let (out, count) = expand_arrow_sequences(code);
    assert_eq!(count, 0);
    assert_eq!(out, code);
}

#[test]
fn expands_arrow_sequence_with_regex_commas_safely() {
    let code = r"const f = () => (/a,b/.test(x), work(), done());";
    let (out, count) = expand_arrow_sequences(code);
    assert_eq!(count, 1);
    assert!(out.contains(r"/a,b/.test(x);"), "regex literal must stay intact: {out}");
    assert!(out.contains("work();"), "Should split second part: {out}");
    assert!(out.contains("return done();"), "Should return final part: {out}");
}

#[test]
fn ignores_arrow_text_inside_nested_template_literal() {
    let code = "const s = `${`=> (a(), b(), c());`}`;";
    let (out, count) = expand_arrow_sequences(code);
    assert_eq!(count, 0);
    assert_eq!(out, code);
}
