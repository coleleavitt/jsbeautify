use super::*;
use oxc_allocator::Allocator;
use oxc_parser::Parser;
use oxc_span::SourceType;

#[test]
fn detects_simple_dispatcher() {
    let code = r"
            function F(s, a) {
                switch (s) {
                    case X: { foo(); } break;
                    case Y: { bar(); } break;
                }
            }
        ";
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
    let detector = DispatcherDetector::new();
    let map = detector.detect(&ret.program);

    assert_eq!(map.len(), 1);
    let info = map.get("F").expect("should detect F");
    assert_eq!(info.state_param, "s");
    assert_eq!(info.args_param, "a");
    assert_eq!(info.cases.len(), 2);
    assert!(info.cases.contains_key("X"));
    assert!(info.cases.contains_key("Y"));
    assert!(!info.has_default);
    assert!(info.alt_name.is_none());
}

#[test]
fn detects_var_assigned_dispatcher() {
    let code = r"
            var F = function G(s, a) {
                switch (s) {
                    case X: { foo(); } break;
                }
            };
        ";
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
    let detector = DispatcherDetector::new();
    let map = detector.detect(&ret.program);

    let info = map.get("F").expect("should detect F");
    assert_eq!(info.name, "F");
    assert_eq!(info.alt_name.as_deref(), Some("G"));
    assert_eq!(info.cases.len(), 1);
    assert!(info.cases.contains_key("X"));
}

#[test]
fn ignores_non_dispatcher() {
    let code = "function f(x) { return x + 1; }";
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
    let detector = DispatcherDetector::new();
    let map = detector.detect(&ret.program);

    assert!(map.is_empty());
}
