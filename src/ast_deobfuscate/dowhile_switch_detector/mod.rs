//! Do-while-switch dispatcher detector.
//!
//! Scans a JavaScript AST for dispatcher functions of the form:
//! `function F(STATE, ARGS) { do { switch(STATE) { ... } } while (STATE != SENTINEL); }`
//!
//! This is a read-only scan (no AST mutation). It builds a map of dispatcher
//! names to their [`DoWhileDispatcherInfo`] metadata.

fn dwsd_dbg_enabled() -> bool {
    std::env::var("AST_DWSD_DEBUG").is_ok()
}

macro_rules! dwsd_dbg {
    ($($arg:tt)*) => {
        if crate::ast_deobfuscate::dowhile_switch_detector::dwsd_dbg_enabled() { eprintln!($($arg)*); }
    };
}

mod constants;
mod deferred_const;
mod detector;
mod extraction;
mod transition;
mod types;
mod walker;

pub use constants::{collect_constants, resolve_compound_transitions};
pub use detector::DoWhileSwitchDetector;
pub use types::{CompoundOp, DoWhileCaseInfo, DoWhileDispatcherInfo, DoWhileDispatcherMap, StateTransition};

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_span::SourceType;
    use rustc_hash::FxHashMap;

    #[test]
    fn detects_simple_dowhile_switch() {
        let code = r"
            function F(s, a) {
                do {
                    switch (s) {
                        case X: { s = Y; } break;
                        case Y: { return 1; } break;
                    }
                } while (s != Z);
            }
        ";
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let detector = DoWhileSwitchDetector::new();
        let map = detector.detect(&ret.program);

        assert_eq!(map.len(), 1);
        let info = map.get("F").expect("should detect F");
        assert_eq!(info.state_param, "s");
        assert_eq!(info.args_param, "a");
        assert_eq!(info.exit_sentinel, "Z");
        assert_eq!(info.cases.len(), 2);
        assert!(!info.has_default);
    }

    #[test]
    fn detects_var_assigned_dowhile() {
        let code = r"
            var F = function G(s, a) {
                do {
                    switch (s) {
                        case X: { s = Y; } break;
                    }
                } while (s != Z);
            };
        ";
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let detector = DoWhileSwitchDetector::new();
        let map = detector.detect(&ret.program);

        let info = map.get("F").expect("should detect F");
        assert_eq!(info.name, "F");
        assert_eq!(info.alt_name.as_deref(), Some("G"));
        assert!(map.contains_key("G"));
    }

    #[test]
    fn classifies_sequential_transition() {
        let code = r"
            function F(s, a) {
                do {
                    switch (s) {
                        case X: { foo(); s = NEXT; } break;
                    }
                } while (s != Z);
            }
        ";
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let detector = DoWhileSwitchDetector::new();
        let map = detector.detect(&ret.program);

        let info = map.get("F").expect("should detect F");
        assert_eq!(info.cases.len(), 1);
        assert_eq!(
            info.cases[0].transition,
            StateTransition::Sequential("NEXT".to_string())
        );
    }

    #[test]
    fn classifies_return_transition() {
        let code = r"
            function F(s, a) {
                do {
                    switch (s) {
                        case X: { return 42; } break;
                    }
                } while (s != Z);
            }
        ";
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let detector = DoWhileSwitchDetector::new();
        let map = detector.detect(&ret.program);

        let info = map.get("F").expect("should detect F");
        assert_eq!(info.cases.len(), 1);
        assert_eq!(info.cases[0].transition, StateTransition::Return);
    }

    #[test]
    fn classifies_conditional_with_both_labels() {
        let code = r"
            function F(s, a) {
                do {
                    switch (s) {
                        case X: {
                            if (a[0]) { s = A; } else { s = B; }
                        } break;
                    }
                } while (s != Z);
            }
        ";
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let detector = DoWhileSwitchDetector::new();
        let map = detector.detect(&ret.program);

        let info = map.get("F").expect("should detect F");
        assert_eq!(info.cases.len(), 1);
        assert_eq!(
            info.cases[0].transition,
            StateTransition::Conditional("A".to_string(), "B".to_string())
        );
    }

    #[test]
    fn classifies_compound_add_assign() {
        let code = r"
            function F(s, a) {
                do {
                    switch (s) {
                        case X: { foo(); s += C; } break;
                    }
                } while (s != Z);
            }
        ";
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let detector = DoWhileSwitchDetector::new();
        let map = detector.detect(&ret.program);

        let info = map.get("F").expect("should detect F");
        assert_eq!(
            info.cases[0].transition,
            StateTransition::Compound {
                op: CompoundOp::AddAssign,
                rhs_name: "C".to_string()
            }
        );
    }

    #[test]
    fn classifies_compound_sub_assign() {
        let code = r"
            function F(s, a) {
                do {
                    switch (s) {
                        case X: { bar(); s -= D; } break;
                    }
                } while (s != Z);
            }
        ";
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let detector = DoWhileSwitchDetector::new();
        let map = detector.detect(&ret.program);

        let info = map.get("F").expect("should detect F");
        assert_eq!(
            info.cases[0].transition,
            StateTransition::Compound {
                op: CompoundOp::SubAssign,
                rhs_name: "D".to_string()
            }
        );
    }

    #[test]
    fn resolves_compound_to_sequential() {
        let mut info = DoWhileDispatcherInfo {
            name: "F".to_string(),
            alt_name: None,
            state_param: "s".to_string(),
            args_param: "a".to_string(),
            exit_sentinel: "Z".to_string(),
            cases: vec![
                DoWhileCaseInfo {
                    label: "X".to_string(),
                    body_statement_count: 1,
                    transition: StateTransition::Compound {
                        op: CompoundOp::AddAssign,
                        rhs_name: "C".to_string(),
                    },
                },
                DoWhileCaseInfo {
                    label: "Y".to_string(),
                    body_statement_count: 1,
                    transition: StateTransition::Return,
                },
            ],
            has_default: false,
        };
        let mut constants: FxHashMap<String, i64> = FxHashMap::default();
        constants.insert("X".to_string(), 10);
        constants.insert("C".to_string(), 3);
        constants.insert("Y".to_string(), 13);

        let resolved = resolve_compound_transitions(&mut info, &constants);
        assert_eq!(resolved, 1);
        assert_eq!(info.cases[0].transition, StateTransition::Sequential("Y".to_string()));
    }

    #[test]
    fn unresolved_compound_stays() {
        let mut info = DoWhileDispatcherInfo {
            name: "F".to_string(),
            alt_name: None,
            state_param: "s".to_string(),
            args_param: "a".to_string(),
            exit_sentinel: "Z".to_string(),
            cases: vec![DoWhileCaseInfo {
                label: "X".to_string(),
                body_statement_count: 1,
                transition: StateTransition::Compound {
                    op: CompoundOp::AddAssign,
                    rhs_name: "UNKNOWN".to_string(),
                },
            }],
            has_default: false,
        };
        let mut constants: FxHashMap<String, i64> = FxHashMap::default();
        constants.insert("X".to_string(), 10);

        let resolved = resolve_compound_transitions(&mut info, &constants);
        assert_eq!(resolved, 0);
        assert!(matches!(info.cases[0].transition, StateTransition::Compound { .. }));
    }
}
