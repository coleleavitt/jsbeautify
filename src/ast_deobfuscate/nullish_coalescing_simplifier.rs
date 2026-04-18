//! Nullish coalescing (??) chain simplification
//!
//! Detects and simplifies nullish coalescing chains.
//! Identifies patterns where multiple ?? operators are chained.
//!
//! Pattern:
//! ```javascript
//! a ?? b ?? c ?? default
//! x ?? y ?? null ?? z
//! value ?? undefined ?? fallback
//! ```
//!
//! This pass detects nullish coalescing patterns and counts them.

use oxc_ast::ast::{Expression, LogicalExpression, LogicalOperator};
use oxc_traverse::{Traverse, TraverseCtx};

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct NullishCoalescingSimplifier {
    detected_count: usize,
}

impl NullishCoalescingSimplifier {
    #[must_use]
    pub const fn new() -> Self {
        Self { detected_count: 0 }
    }

    #[must_use]
    pub const fn detected_count(&self) -> usize {
        self.detected_count
    }

    /// Check if a logical expression is a nullish coalescing operator
    fn is_nullish_coalescing(expr: &LogicalExpression) -> bool {
        matches!(expr.operator, LogicalOperator::Coalesce)
    }

    /// Check if left side is also a nullish coalescing chain
    fn has_nullish_left(expr: &LogicalExpression<'_>) -> bool {
        matches!(&expr.left, Expression::LogicalExpression(log_expr) if Self::is_nullish_coalescing(log_expr))
    }
}

impl Default for NullishCoalescingSimplifier {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for NullishCoalescingSimplifier {
    fn enter_logical_expression(&mut self, expr: &mut LogicalExpression<'a>, _ctx: &mut Ctx<'a>) {
        if Self::is_nullish_coalescing(expr) && Self::has_nullish_left(expr) {
            eprintln!("[NULLISH_COALESCING] Detected nullish coalescing chain");
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

        let mut simplifier = NullishCoalescingSimplifier::new();
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

        traverse_mut_with_ctx(&mut simplifier, &mut program, &mut ctx);

        simplifier.detected_count()
    }

    #[test]
    fn test_detect_simple_nullish() {
        let code = "a ?? b";
        let count = run_detect(code);
        assert_eq!(count, 0, "Should NOT count single ??, got: {count}");
    }

    #[test]
    fn test_detect_double_nullish() {
        let code = "a ?? b ?? c";
        let count = run_detect(code);
        assert!(count >= 1, "Should detect double ??, got: {count}");
    }

    #[test]
    fn test_detect_triple_nullish() {
        let code = "a ?? b ?? c ?? d";
        let count = run_detect(code);
        assert!(count >= 1, "Should detect triple ??, got: {count}");
    }

    #[test]
    fn test_detect_nullish_with_null() {
        let code = "x ?? null ?? z";
        let count = run_detect(code);
        assert!(count >= 1, "Should detect ?? with null, got: {count}");
    }

    #[test]
    fn test_detect_nullish_with_undefined() {
        let code = "value ?? undefined ?? fallback";
        let count = run_detect(code);
        assert!(count >= 1, "Should detect ?? with undefined, got: {count}");
    }

    #[test]
    fn test_ignore_logical_or() {
        let code = "a || b || c";
        let count = run_detect(code);
        assert_eq!(count, 0, "Should NOT detect ||, got: {count}");
    }

    #[test]
    fn test_ignore_logical_and() {
        let code = "a && b && c";
        let count = run_detect(code);
        assert_eq!(count, 0, "Should NOT detect &&, got: {count}");
    }

    #[test]
    fn test_detect_multiple_chains() {
        let code = "a ?? b ?? c; x ?? y ?? z ?? w";
        let count = run_detect(code);
        assert!(count >= 2, "Should detect multiple chains, got: {count}");
    }
}
