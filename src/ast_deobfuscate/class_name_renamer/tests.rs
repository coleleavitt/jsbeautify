use super::*;
use crate::ast_deobfuscate::{DeobfuscateState, semantic};
#[cfg(test)]
use oxc_allocator::Allocator;
use oxc_parser::Parser;
use oxc_span::SourceType;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

fn collect(code: &str) -> ClassNameMap {
    let alloc = Allocator::default();
    let parsed = Parser::new(&alloc, code, SourceType::mjs()).parse();
    let mut prog = parsed.program;
    let mut collector = ClassNameCollector::new();
    let scoping = semantic::scoping(&prog);
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &alloc);
    traverse_mut_with_ctx(&mut collector, &mut prog, &mut ctx);
    collector.into_map()
}

fn collect_display(code: &str) -> ClassNameMap {
    let alloc = Allocator::default();
    let parsed = Parser::new(&alloc, code, SourceType::mjs()).parse();
    let mut prog = parsed.program;
    let mut collector = DisplayNameCollector::new();
    let scoping = semantic::scoping(&prog);
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &alloc);
    traverse_mut_with_ctx(&mut collector, &mut prog, &mut ctx);
    collector.into_map()
}

#[test]
fn dotted_display_names_are_sanitized_into_legal_identifiers() {
    // React compound components (`Accordion.Content`) would otherwise be
    // written into a binding position as `var Accordion.Content = ...`, which
    // is invalid JS and aborts deobfuscation of the entire file.
    let code = r#"
            var C = forwardRef(function () {});
            C.displayName = "Accordion.Content";
        "#;
    let names: Vec<String> = collect_display(code).into_values().collect();
    assert_eq!(names, vec!["Accordion_Content".to_string()]);
}

#[test]
fn detects_this_name_in_constructor() {
    let code = r#"
            class c5 extends Error {
                constructor(H) {
                    super(H);
                    this.name = "AbortError";
                }
            }
        "#;
    let map = collect(code);
    assert!(map.values().any(|name| name == "AbortError"));
}

#[test]
fn detects_this_name_in_a_joined_constructor_statement() {
    // Minifiers join `super(H); this.name = "AbortError";` into one sequence.
    let code = r#"var c5 = class extends Error { constructor(H) { super(H), this.name = "AbortError"; } };"#;
    let map = collect(code);
    assert!(map.values().any(|name| name == "AbortError"), "{map:?}");
}

#[test]
fn skips_xml_names() {
    let code = r#"
            class x1 {
                constructor() {
                    this.name = "?xml";
                }
            }
        "#;
    let map = collect(code);
    assert!(map.is_empty());
}

#[test]
fn skips_already_readable() {
    let code = r#"
            class AbortError extends Error {
                constructor(H) {
                    super(H);
                    this.name = "AbortError";
                }
            }
        "#;
    let map = collect(code);
    assert!(map.is_empty());
}
