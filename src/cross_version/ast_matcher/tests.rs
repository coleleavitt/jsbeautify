use super::*;

#[test]
fn test_statement_extraction() {
    let matcher = StatementMatcher::default();
    let source = "var x = 1;\nvar y = 2;\nfunction foo() { return x + y; }";
    let stmts = matcher.extract_statements(source);
    assert_eq!(stmts.len(), 3);
}

#[test]
fn test_identical_statements_match() {
    let matcher = StatementMatcher::default();
    let source = "var x = 1;";
    let stmts1 = matcher.extract_statements(source);
    let stmts2 = matcher.extract_statements(source);

    let statement_matches = matcher.match_statements(&stmts1, &stmts2);
    assert_eq!(statement_matches.len(), 1);
}

#[test]
fn test_different_var_names_same_structure() {
    let matcher = StatementMatcher::default();
    let source1 = "var abc = 1;";
    let source2 = "var xyz = 1;";
    let stmts1 = matcher.extract_statements(source1);
    let stmts2 = matcher.extract_statements(source2);

    assert_eq!(stmts1[0].hash, stmts2[0].hash);
}

#[test]
fn test_identifier_collection() {
    let matcher = StatementMatcher::default();
    let source = "var x = y + z;";
    let stmts = matcher.extract_statements(source);
    assert_eq!(stmts.len(), 1);
    assert_eq!(stmts[0].identifiers.len(), 3);
}
