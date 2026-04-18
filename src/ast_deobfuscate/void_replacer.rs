use oxc_ast::ast::{Expression, IdentifierReference, UnaryOperator};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use oxc_traverse::{Traverse, TraverseCtx};
use std::cell::Cell;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct VoidReplacer {
    changed: bool,
}

impl VoidReplacer {
    #[must_use]
    pub const fn new() -> Self {
        Self { changed: false }
    }

    #[must_use]
    pub const fn has_changed(&self) -> bool {
        self.changed
    }
}

impl Default for VoidReplacer {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for VoidReplacer {
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        let Expression::UnaryExpression(unary) = expr else {
            return;
        };
        if unary.operator != UnaryOperator::Void {
            return;
        }
        if !is_pure_operand(&unary.argument) {
            return;
        }
        eprintln!("[AST] Converting void <pure-expr> -> undefined");
        self.changed = true;
        *expr = Expression::Identifier(ctx.ast.alloc(IdentifierReference {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            name: ctx.ast.ident("undefined"),
            reference_id: Cell::default(),
        }));
    }
}

fn is_pure_operand(expr: &Expression<'_>) -> bool {
    match expr {
        Expression::NumericLiteral(_)
        | Expression::StringLiteral(_)
        | Expression::BooleanLiteral(_)
        | Expression::NullLiteral(_)
        | Expression::BigIntLiteral(_)
        | Expression::RegExpLiteral(_) => true,
        Expression::Identifier(id) => id.name.as_str() == "undefined",
        Expression::ArrayExpression(a) => a.elements.is_empty(),
        Expression::ObjectExpression(o) => o.properties.is_empty(),
        Expression::ParenthesizedExpression(p) => is_pure_operand(&p.expression),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast_deobfuscate::DeobfuscateState;
    use oxc_allocator::Allocator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_semantic::SemanticBuilder;
    use oxc_span::SourceType;
    use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

    fn run_void_replacer(code: &str) -> String {
        let allocator = Allocator::default();
        let source_type = SourceType::mjs();
        let ret = Parser::new(&allocator, code, source_type).parse();
        let mut program = ret.program;

        let mut replacer = VoidReplacer::new();
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

        traverse_mut_with_ctx(&mut replacer, &mut program, &mut ctx);

        Codegen::new().build(&program).code
    }

    #[test]
    fn test_void_zero_to_undefined() {
        let output = run_void_replacer("var x = void 0;");
        eprintln!("Output: {output}");
        assert!(output.contains("undefined"), "Expected undefined, got: {output}");
        assert!(!output.contains("void"), "Should not contain void, got: {output}");
    }

    #[test]
    fn test_void_literal_to_undefined() {
        for src in [
            "var x = void 5;",
            "var x = void \"hi\";",
            "var x = void (0);",
            "var x = void []",
        ] {
            let output = run_void_replacer(src);
            assert!(output.contains("undefined"), "{src} -> {output}");
        }
    }

    #[test]
    fn test_preserve_void_side_effect() {
        let output = run_void_replacer("var x = void f();");
        assert!(
            output.contains("void"),
            "must preserve side-effecting void, got: {output}"
        );
    }

    #[test]
    fn test_multiple_void_zero() {
        let output = run_void_replacer("var x = void 0, y = void 0;");
        eprintln!("Output: {output}");
        let count = output.matches("undefined").count();
        assert!(count >= 2, "Expected two undefined, got {count} in: {output}");
    }
}
