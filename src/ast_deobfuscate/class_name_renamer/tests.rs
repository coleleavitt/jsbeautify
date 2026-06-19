#[cfg(test)]
use super::*;
use crate::ast_deobfuscate::DeobfuscateState;
use oxc_allocator::Allocator;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

fn collect(code: &str) -> ClassNameMap {
    let alloc = Allocator::default();
    let parsed = Parser::new(&alloc, code, SourceType::mjs()).parse();
    let mut prog = parsed.program;
    let mut collector = ClassNameCollector::new();
    let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &alloc);
    traverse_mut_with_ctx(&mut collector, &mut prog, &mut ctx);
    collector.into_map()
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
