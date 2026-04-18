//! Optional chaining (?.) normalization
//!
//! Detects and normalizes optional chaining expressions.
//! Converts chains of optional accesses to simpler forms when possible.
//!
//! Pattern:
//! ```javascript
//! obj?.prop
//! obj?.method?.()
//! obj?.[key]
//! obj?.prop?.method?.()
//! ```
//!
//! This pass detects optional chaining patterns and counts them.
//! Full normalization requires semantic analysis to determine if
//! the optional chaining is necessary.

use oxc_ast::ast::{ChainElement, ChainExpression, Expression};
use oxc_traverse::{Traverse, TraverseCtx};

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct OptionalChainingNormalizer {
    detected_count: usize,
}

impl OptionalChainingNormalizer {
    #[must_use]
    pub const fn new() -> Self {
        Self { detected_count: 0 }
    }

    #[must_use]
    pub const fn detected_count(&self) -> usize {
        self.detected_count
    }

    /// Count the depth of optional chaining
    fn count_chain_depth(_chain: &ChainExpression) -> usize {
        // For now, just return 1 to indicate a chain was detected
        // Full depth analysis would require traversing the chain structure
        1
    }
}

impl Default for OptionalChainingNormalizer {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for OptionalChainingNormalizer {
    fn enter_expression(&mut self, expr: &mut Expression<'a>, _ctx: &mut Ctx<'a>) {
        if let Expression::ChainExpression(chain) = expr {
            let depth = Self::count_chain_depth(chain);
            eprintln!("[OPTIONAL_CHAINING] Detected optional chaining with depth {}", depth);
            self.detected_count += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast_deobfuscate::DeobfuscateState;
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_semantic::SemanticBuilder;
    use oxc_span::SourceType;
    use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

    fn run_detect(code: &str) -> usize {
        let allocator = Allocator::default();
        let source_type = SourceType::mjs();
        let ret = Parser::new(&allocator, code, source_type).parse();
        let mut program = ret.program;

        let mut normalizer = OptionalChainingNormalizer::new();
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

        traverse_mut_with_ctx(&mut normalizer, &mut program, &mut ctx);

        normalizer.detected_count()
    }

    #[test]
    fn test_detect_optional_property() {
        let code = "obj?.prop";
        let count = run_detect(code);
        assert_eq!(count, 1, "Should detect optional property access, got: {count}");
    }

    #[test]
    fn test_detect_optional_method_call() {
        let code = "obj?.method?.()";
        let count = run_detect(code);
        assert!(count >= 1, "Should detect optional method call, got: {count}");
    }

    #[test]
    fn test_detect_optional_computed() {
        let code = "obj?.[key]";
        let count = run_detect(code);
        assert_eq!(count, 1, "Should detect optional computed access, got: {count}");
    }

    #[test]
    fn test_detect_chained_optional() {
        let code = "obj?.prop?.method?.()";
        let count = run_detect(code);
        assert!(count >= 1, "Should detect chained optional access, got: {count}");
    }

    #[test]
    fn test_ignore_regular_access() {
        let code = "obj.prop";
        let count = run_detect(code);
        assert_eq!(count, 0, "Should NOT detect regular property access, got: {count}");
    }

    #[test]
    fn test_ignore_regular_call() {
        let code = "obj.method()";
        let count = run_detect(code);
        assert_eq!(count, 0, "Should NOT detect regular method call, got: {count}");
    }

    #[test]
    fn test_detect_multiple_chains() {
        let code = "a?.b; c?.d?.e; f?.[g]";
        let count = run_detect(code);
        assert!(count >= 2, "Should detect multiple chains, got: {count}");
    }
}
