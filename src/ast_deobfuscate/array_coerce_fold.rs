//! Array-to-string coercion constant folder.
//!
//! Replaces JavaScript array coercion patterns used by Akamai BMP:
//! - `[] + []`       → `""`
//! - `[] + undefined` → `"undefined"`
//! - `undefined + []` → `"undefined"`

use oxc_ast::ast::{BinaryOperator, Expression, StringLiteral};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use oxc_traverse::{Traverse, TraverseCtx};
use std::cell::Cell;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

fn is_empty_array(expr: &Expression<'_>) -> bool {
    matches!(expr, Expression::ArrayExpression(a) if a.elements.is_empty())
}

fn is_undefined(expr: &Expression<'_>) -> bool {
    matches!(expr, Expression::Identifier(id) if id.name.as_str() == "undefined")
}

pub struct ArrayCoerceFold {
    folded: usize,
}

impl ArrayCoerceFold {
    #[must_use]
    pub const fn new() -> Self {
        Self { folded: 0 }
    }

    #[must_use]
    pub const fn folded(&self) -> usize {
        self.folded
    }
}

impl Default for ArrayCoerceFold {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for ArrayCoerceFold {
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        let Expression::BinaryExpression(bin) = expr else {
            return;
        };
        if bin.operator != BinaryOperator::Addition {
            return;
        }

        let replacement = if is_empty_array(&bin.left) && is_empty_array(&bin.right) {
            ""
        } else if is_empty_array(&bin.left) && is_undefined(&bin.right) {
            "undefined"
        } else if is_undefined(&bin.left) && is_empty_array(&bin.right) {
            "undefined"
        } else {
            return;
        };

        if self.folded < 10 {
            eprintln!("[AST/array-coerce] folding [] + [] -> {:?}", replacement);
        }
        self.folded += 1;

        *expr = Expression::StringLiteral(ctx.ast.alloc(StringLiteral {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            value: ctx.ast.str(replacement),
            raw: None,
            lone_surrogates: false,
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_allocator::Allocator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_semantic::SemanticBuilder;
    use oxc_span::SourceType;
    use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

    fn run(code: &str) -> String {
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let mut program = ret.program;
        let mut pass = ArrayCoerceFold::new();
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);
        traverse_mut_with_ctx(&mut pass, &mut program, &mut ctx);
        Codegen::new().build(&program).code
    }

    #[test]
    fn folds_empty_array_plus_empty_array() {
        let out = run("var x = [] + [];");
        assert!(out.contains("\"\""), "got: {out}");
    }

    #[test]
    fn folds_empty_array_plus_undefined() {
        let out = run("var x = [] + undefined;");
        assert!(out.contains("\"undefined\""), "got: {out}");
    }

    #[test]
    fn folds_undefined_plus_empty_array() {
        let out = run("var x = undefined + [];");
        assert!(out.contains("\"undefined\""), "got: {out}");
    }

    #[test]
    fn preserves_non_empty_array() {
        let out = run("var x = [1] + [];");
        assert!(out.contains("[1]"), "got: {out}");
    }
}
