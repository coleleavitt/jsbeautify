use oxc_allocator::CloneIn;
use oxc_ast::ast::{
    BinaryOperator, ComputedMemberExpression, Expression, IdentifierName, MemberExpression, StaticMemberExpression,
};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use oxc_traverse::{Traverse, TraverseCtx};
use std::cell::Cell;

use crate::ast_deobfuscate::pass::flag_pass;
use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct DynamicPropertyConverter {
    changed: bool,
}

impl DynamicPropertyConverter {
    fn try_convert<'a>(
        &mut self,
        computed: &ComputedMemberExpression<'a>,
        ctx: &mut Ctx<'a>,
    ) -> Option<Expression<'a>> {
        let property_name = Self::try_extract_property_name(&computed.expression)?;

        if !is_valid_identifier(&property_name) {
            return None;
        }

        eprintln!("[AST] Converting dynamic property [\"{property_name}\"] -> .{property_name}");
        self.changed = true;

        let object = Self::clone_expression(&computed.object, ctx);

        Some(Expression::StaticMemberExpression(ctx.ast.alloc(
            StaticMemberExpression {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                object,
                property: IdentifierName {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                    name: ctx.ast.ident(&property_name),
                },
                optional: false,
            },
        )))
    }

    fn try_extract_property_name(expr: &Expression<'_>) -> Option<String> {
        match expr {
            Expression::StringLiteral(s) => Some(s.value.as_str().to_string()),
            Expression::NumericLiteral(n) => {
                let val = n.value as u32;
                if val <= 127 {
                    char::from_u32(val).map(|c| c.to_string())
                } else {
                    None
                }
            }
            Expression::BinaryExpression(binary) if binary.operator == BinaryOperator::Addition => {
                let left = Self::try_extract_property_name(&binary.left)?;
                let right = Self::try_extract_property_name(&binary.right)?;
                Some(format!("{left}{right}"))
            }
            Expression::ParenthesizedExpression(paren) => Self::try_extract_property_name(&paren.expression),
            _ => None,
        }
    }

    fn clone_expression<'a>(expr: &Expression<'a>, ctx: &mut Ctx<'a>) -> Expression<'a> {
        expr.clone_in_with_semantic_ids(ctx.ast.allocator)
    }
}

flag_pass!(DynamicPropertyConverter, changed, has_changed);

fn is_valid_identifier(s: &str) -> bool {
    if s.is_empty() || s.len() > 100 {
        return false;
    }

    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    if !first.is_alphabetic() && first != '_' && first != '$' {
        return false;
    }

    for ch in chars {
        if !ch.is_alphanumeric() && ch != '_' && ch != '$' {
            return false;
        }
    }

    true
}

impl<'a> Traverse<'a, DeobfuscateState> for DynamicPropertyConverter {
    fn exit_member_expression(&mut self, member: &mut MemberExpression<'a>, ctx: &mut Ctx<'a>) {
        if let MemberExpression::ComputedMemberExpression(computed) = member
            && let Some(converted) = self.try_convert(computed, ctx)
            && let Expression::StaticMemberExpression(static_member) = converted
        {
            *member = MemberExpression::StaticMemberExpression(static_member);
        }
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

    fn run_dynamic_property(code: &str) -> String {
        let allocator = Allocator::default();
        let source_type = SourceType::mjs();
        let ret = Parser::new(&allocator, code, source_type).parse();
        let mut program = ret.program;

        let mut converter = DynamicPropertyConverter::new();
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

        traverse_mut_with_ctx(&mut converter, &mut program, &mut ctx);

        Codegen::new().build(&program).code
    }

    #[test]
    fn test_convert_string_literal_property() {
        let output = run_dynamic_property(r#"obj["property"]"#);
        eprintln!("Output: {output}");
        assert!(output.contains("obj.property"), "Expected obj.property, got: {output}");
    }

    #[test]
    fn test_convert_hex_ascii() {
        let output = run_dynamic_property("obj[97]");
        eprintln!("Output: {output}");
        assert!(output.contains("obj.a"), "Expected obj.a (97 = 'a'), got: {output}");
    }

    #[test]
    fn test_convert_concatenated_property() {
        let output = run_dynamic_property(r#"obj["pro" + "perty"]"#);
        eprintln!("Output: {output}");
        assert!(output.contains("obj.property"), "Expected obj.property, got: {output}");
    }

    #[test]
    fn test_preserve_dynamic_property() {
        let output = run_dynamic_property("obj[variable]");
        eprintln!("Output: {output}");
        assert!(
            output.contains("obj[variable]"),
            "Should preserve dynamic access, got: {output}"
        );
    }

    #[test]
    fn test_preserve_invalid_identifier() {
        let output = run_dynamic_property(r#"obj["123invalid"]"#);
        eprintln!("Output: {output}");
        assert!(
            output.contains('[') && output.contains("123invalid"),
            "Should preserve invalid identifier, got: {output}"
        );
    }
}
