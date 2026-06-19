use super::SourcemapParser;
use super::vlq::decode_vlq;

#[test]
fn test_decode_vlq_simple() {
    assert_eq!(decode_vlq("A").unwrap(), vec![0]);
    assert_eq!(decode_vlq("C").unwrap(), vec![1]);
    assert_eq!(decode_vlq("D").unwrap(), vec![-1]);
}

#[test]
fn test_decode_vlq_multi() {
    let values = decode_vlq("AAAA").unwrap();
    assert_eq!(values.len(), 4);
}

#[test]
fn test_ast_identifier_extraction() {
    let parser = SourcemapParser::new();
    let source = "var foo = bar;";
    let ids = parser.extract_identifiers_ast(source);
    assert!(ids.iter().any(|(_, _, name)| name == "foo"));
    assert!(ids.iter().any(|(_, _, name)| name == "bar"));
}
