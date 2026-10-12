use oxc_allocator::{CloneIn, GetAllocator};
use oxc_ast::ast::{BinaryOperator, ComputedMemberExpression, Expression, MemberExpression};
use oxc_span::SPAN;
use oxc_traverse::{Traverse, TraverseCtx};

use crate::ast_deobfuscate::pass::counter_pass;
use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct DynamicPropertyConverter {
    rewrites: usize,
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

        deob_trace!("[AST] Converting dynamic property [\"{property_name}\"] -> .{property_name}");
        self.rewrites += 1;

        let object = Self::clone_expression(&computed.object, ctx);

        let property = oxc_ast::ast::IdentifierName::new(SPAN, ctx.allocator().alloc_str(&property_name), ctx);
        Some(Expression::new_static_member_expression(
            SPAN,
            object,
            property,
            computed.optional,
            ctx,
        ))
    }

    fn try_extract_property_name(expr: &Expression<'_>) -> Option<String> {
        match expr {
            Expression::StringLiteral(s) => Some(s.value.as_str().to_string()),
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
        expr.clone_in_with_semantic_ids(ctx.allocator())
    }
}

counter_pass!(DynamicPropertyConverter, rewrites, rewrites);

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
    fn numeric_key_names_its_decimal_string() {
        for code in ["obj[97]", r#"obj["a" + 1 + 2]"#] {
            let output = run_dynamic_property(code);
            assert!(output.contains('['), "numeric keys are not characters: {output}");
        }
    }

    #[test]
    fn test_convert_concatenated_property() {
        let output = run_dynamic_property(r#"obj["pro" + "perty"]"#);
        eprintln!("Output: {output}");
        assert!(output.contains("obj.property"), "Expected obj.property, got: {output}");
    }

    #[test]
    fn keeps_optional_flag_on_converted_members() {
        let output = run_dynamic_property(r#"console.log(u[k]?.["city"] ?? "-", u.p?.["a"]?.["b"]);"#);
        assert!(output.contains("u[k]?.city"), "got: {output}");
        assert!(output.contains("u.p?.a?.b"), "got: {output}");
    }

    #[test]
    fn non_optional_link_inside_chain_stays_non_optional() {
        let output = run_dynamic_property(r#"a?.["b"]["c"]?.["d"]();"#);
        assert!(output.contains("a?.b.c?.d()"), "got: {output}");
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
