use oxc_allocator::Allocator;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

use crate::ast_deobfuscate::esbuild_helper::*;
use crate::ast_deobfuscate::state::DeobfuscateState;

mod annotations_and_core;
mod async_spread;
mod decorators_modern;
mod private_and_tslib;
mod runtime_misc;

fn run_collector(code: &str) -> EsbuildHelperCollector {
    let allocator = Allocator::default();
    let source_type = SourceType::mjs();
    let ret = Parser::new(&allocator, code, source_type).parse();
    let mut program = ret.program;

    let mut collector = EsbuildHelperCollector::new();
    let state = DeobfuscateState::new();
    let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

    traverse_mut_with_ctx(&mut collector, &mut program, &mut ctx);
    collector
}
