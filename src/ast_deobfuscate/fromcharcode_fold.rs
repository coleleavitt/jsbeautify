//! `String.fromCharCode` constant folder.
//!
//! Replaces `String.fromCharCode(72, 101, 108, 108, 111)` with the literal
//! `"Hello"` whenever every argument is an integer numeric literal in the
//! valid UTF-16 code unit range (0..=0xFFFF).
//!
//! If ANY argument is not a valid integer in range the call is left intact —
//! partial folding would change semantics.

use oxc_ast::ast::{Expression, StringLiteral};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use oxc_traverse::{Traverse, TraverseCtx};
use std::cell::Cell;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct FromCharCodeFolder {
    folded: usize,
}

impl FromCharCodeFolder {
    #[must_use]
    pub const fn new() -> Self {
        Self { folded: 0 }
    }

    #[must_use]
    pub const fn folded(&self) -> usize {
        self.folded
    }
}

impl Default for FromCharCodeFolder {
    fn default() -> Self {
        Self::new()
    }
}

fn is_string_from_charcode(expr: &Expression<'_>) -> bool {
    let Expression::StaticMemberExpression(sme) = expr else {
        return false;
    };
    if sme.property.name.as_str() != "fromCharCode" {
        return false;
    }
    let Expression::Identifier(obj) = &sme.object else {
        return false;
    };
    obj.name.as_str() == "String"
}

fn extract_codepoint(expr: &Expression<'_>) -> Option<u32> {
    let val = match expr {
        Expression::NumericLiteral(n) => n.value,
        Expression::UnaryExpression(u) if matches!(u.operator, oxc_ast::ast::UnaryOperator::UnaryPlus) => {
            if let Expression::NumericLiteral(n) = &u.argument {
                n.value
            } else {
                return None;
            }
        }
        _ => return None,
    };
    if val.fract() != 0.0 || !(0.0..=65535.0).contains(&val) {
        return None;
    }
    Some(val as u32)
}

impl<'a> Traverse<'a, DeobfuscateState> for FromCharCodeFolder {
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        let Expression::CallExpression(call) = expr else {
            return;
        };
        if !is_string_from_charcode(&call.callee) {
            return;
        }
        if call.arguments.is_empty() {
            return;
        }
        let mut chars = Vec::with_capacity(call.arguments.len());
        for arg in &call.arguments {
            let Some(e) = arg.as_expression() else { return };
            let Some(cp) = extract_codepoint(e) else { return };
            let Some(ch) = char::from_u32(cp) else { return };
            chars.push(ch);
        }
        let decoded: String = chars.into_iter().collect();
        if self.folded < 10 {
            let preview: String = decoded.chars().take(32).collect();
            eprintln!(
                "[AST/fromCharCode] folding {} char codes -> {:?}",
                call.arguments.len(),
                preview
            );
        }
        self.folded += 1;
        *expr = Expression::StringLiteral(ctx.ast.alloc(StringLiteral {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            value: ctx.ast.str(&decoded),
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
        let mut pass = FromCharCodeFolder::new();
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);
        traverse_mut_with_ctx(&mut pass, &mut program, &mut ctx);
        Codegen::new().build(&program).code
    }

    #[test]
    fn folds_hello() {
        let out = run("var x = String.fromCharCode(72, 101, 108, 108, 111);");
        assert!(out.contains("\"Hello\""), "got: {out}");
    }

    #[test]
    fn leaves_variable_charcodes_alone() {
        let out = run("var x = String.fromCharCode(a, 65);");
        assert!(out.contains("fromCharCode"), "got: {out}");
    }

    #[test]
    fn rejects_out_of_range() {
        let out = run("var x = String.fromCharCode(65, 70000);");
        assert!(out.contains("fromCharCode"), "got: {out}");
    }

    #[test]
    fn rejects_non_integer() {
        let out = run("var x = String.fromCharCode(65.5);");
        assert!(out.contains("fromCharCode"), "got: {out}");
    }

    #[test]
    fn folds_single_char() {
        let out = run("var x = String.fromCharCode(65);");
        assert!(out.contains("\"A\""), "got: {out}");
    }
}
