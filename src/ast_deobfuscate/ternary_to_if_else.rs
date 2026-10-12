//! Ternary-to-if/else conversion pass
//!
//! Converts standalone ternary expression statements into if/else statements
//! for improved readability. For example:
//!
//! ```js
//! cond ? doA() : doB();
//! ```
//!
//! Becomes:
//!
//! ```js
//! if (cond) {
//!   doA();
//! } else {
//!   doB();
//! }
//! ```
//!
//! Only converts when the `ConditionalExpression` is the direct expression of
//! an `ExpressionStatement`, or the argument of a `return` whose branch is
//! itself a ternary or a comma sequence:
//!
//! ```js
//! return a ? b : c ? d : e;
//! ```
//!
//! Becomes:
//!
//! ```js
//! if (a) {
//!   return b;
//! }
//! return c ? d : e;
//! ```
//!
//! Ternaries in variable declarations, assignments, or any other context are
//! left untouched.

use oxc_allocator::{ArenaVec, CloneIn, GetAllocator};
use oxc_ast::ast::{
    BlockStatement, ConditionalExpression, Expression, FunctionBody, IfStatement, Program, Statement, SwitchCase,
};
use oxc_semantic::ScopeFlags;
use oxc_span::SPAN;
use oxc_traverse::{Ancestor, Traverse, TraverseCtx};

use crate::ast_deobfuscate::pass::counter_pass;
use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct TernaryToIfElse {
    converted_count: usize,
}

counter_pass!(TernaryToIfElse, converted_count, converted_count);

/// `return a ? b : c` reads worse than `if (a) return b; return c;` once a
/// branch is itself a ternary or a comma sequence.
fn split_worthy_return<'s, 'a>(stmt: &'s Statement<'a>) -> Option<&'s ConditionalExpression<'a>> {
    if let Statement::ReturnStatement(ret) = stmt
        && let Some(Expression::ConditionalExpression(cond)) = &ret.argument
        && (is_nested_branch(&cond.consequent) || is_nested_branch(&cond.alternate))
    {
        return Some(cond);
    }
    None
}

fn is_nested_branch(expr: &Expression<'_>) -> bool {
    matches!(
        expr.without_parentheses(),
        Expression::ConditionalExpression(_) | Expression::SequenceExpression(_)
    )
}

pub fn is_convertible(stmt: &Statement<'_>) -> bool {
    matches!(
        stmt,
        Statement::ExpressionStatement(expr_stmt)
            if matches!(expr_stmt.expression, Expression::ConditionalExpression(_))
    ) || split_worthy_return(stmt).is_some()
}

fn block_of<'a>(stmt: Statement<'a>, ctx: &mut Ctx<'a>) -> Statement<'a> {
    let mut body = ArenaVec::new_in(&ctx.ast);
    body.push(stmt);
    let scope_id = ctx.create_child_scope_of_current(ScopeFlags::empty());
    Statement::new_block_statement_with_scope_id(SPAN, body, scope_id, &ctx.ast)
}

/// Build an `IfStatement` from a `ConditionalExpression`, wrapping the
/// consequent and alternate branches in `BlockStatement`s.
fn build_if_stmt<'a>(cond: &ConditionalExpression<'a>, ctx: &mut Ctx<'a>) -> Statement<'a> {
    let test = cond.test.clone_in_with_semantic_ids(ctx.ast.allocator());
    let consequent_expr = cond
        .consequent
        .without_parentheses()
        .clone_in_with_semantic_ids(ctx.ast.allocator());
    let alternate_expr = cond
        .alternate
        .without_parentheses()
        .clone_in_with_semantic_ids(ctx.ast.allocator());

    let consequent = block_of(
        Statement::new_expression_statement(SPAN, consequent_expr, &ctx.ast),
        ctx,
    );
    let alternate = block_of(Statement::new_expression_statement(SPAN, alternate_expr, &ctx.ast), ctx);

    Statement::new_if_statement(SPAN, test, consequent, Some(alternate), &ctx.ast)
}

impl TernaryToIfElse {
    fn convert_statements<'a>(&mut self, stmts: &mut ArenaVec<'a, Statement<'a>>, ctx: &mut Ctx<'a>) {
        if !stmts.iter().any(is_convertible) {
            return;
        }

        let mut new_stmts = ArenaVec::new_in(&ctx.ast);
        for stmt in stmts.iter() {
            if let Statement::ExpressionStatement(expr_stmt) = stmt
                && let Expression::ConditionalExpression(cond) = &expr_stmt.expression
            {
                let mut converted = build_if_stmt(cond, ctx);
                self.convert_branches(&mut converted, ctx);
                new_stmts.push(converted);
                self.converted_count += 1;
                continue;
            }
            if let Some(cond) = split_worthy_return(stmt) {
                let allocator = ctx.ast.allocator();
                let test = cond.test.clone_in_with_semantic_ids(allocator);
                let consequent = cond
                    .consequent
                    .without_parentheses()
                    .clone_in_with_semantic_ids(allocator);
                let alternate = cond
                    .alternate
                    .without_parentheses()
                    .clone_in_with_semantic_ids(allocator);
                let early_return = block_of(Statement::new_return_statement(SPAN, Some(consequent), &ctx.ast), ctx);
                let mut converted = Statement::new_if_statement(SPAN, test, early_return, None, &ctx.ast);
                self.convert_branches(&mut converted, ctx);
                new_stmts.push(converted);
                let mut rest = ArenaVec::new_in(&ctx.ast);
                rest.push(Statement::new_return_statement(SPAN, Some(alternate), &ctx.ast));
                self.convert_statements(&mut rest, ctx);
                new_stmts.extend(rest);
                self.converted_count += 1;
                continue;
            }
            new_stmts.push(stmt.clone_in_with_semantic_ids(ctx.ast.allocator()));
        }
        *stmts = new_stmts;
    }

    /// Converts the blocks just built for a ternary's branches, so nested
    /// ternaries do not wait for another traversal.
    fn convert_branches<'a>(&mut self, stmt: &mut Statement<'a>, ctx: &mut Ctx<'a>) {
        let Statement::IfStatement(if_stmt) = stmt else {
            return;
        };
        let IfStatement {
            consequent, alternate, ..
        } = &mut **if_stmt;
        for branch in std::iter::once(consequent).chain(alternate.as_mut()) {
            if let Statement::BlockStatement(block) = branch {
                self.convert_statements(&mut block.body, ctx);
            }
        }
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for TernaryToIfElse {
    fn exit_program(&mut self, program: &mut Program<'a>, ctx: &mut Ctx<'a>) {
        self.convert_statements(&mut program.body, ctx);
    }

    fn exit_block_statement(&mut self, block: &mut BlockStatement<'a>, ctx: &mut Ctx<'a>) {
        self.convert_statements(&mut block.body, ctx);
    }

    fn exit_function_body(&mut self, body: &mut FunctionBody<'a>, ctx: &mut Ctx<'a>) {
        // Skip arrow expression bodies — converting the ternary there would lose the
        // implicit return value. e.g. `(K) => cond ? a : b` must NOT become
        // `(K) => { if (cond) { a; } else { b; } }` because that changes semantics.
        if let Ancestor::ArrowFunctionExpressionBody(_) = ctx.parent() {
            // FunctionBody is visited only for block-bodied arrows in Oxc 0.148.
            return;
        }
        self.convert_statements(&mut body.statements, ctx);
    }

    fn exit_switch_case(&mut self, case: &mut SwitchCase<'a>, ctx: &mut Ctx<'a>) {
        self.convert_statements(&mut case.consequent, ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast_deobfuscate::{DeobfuscateState, semantic};
    use oxc_allocator::Allocator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_span::SourceType;
    use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

    fn run_convert(code: &str) -> (String, usize) {
        let allocator = Allocator::default();
        let source_type = SourceType::mjs();
        let ret = Parser::new(&allocator, code, source_type).parse();
        let mut program = ret.program;

        let mut converter = TernaryToIfElse::new();
        let state = DeobfuscateState::new();
        let scoping = semantic::scoping(&program);
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

        traverse_mut_with_ctx(&mut converter, &mut program, &mut ctx);

        (Codegen::new().build(&program).code, converter.converted_count())
    }

    #[test]
    fn test_convert_simple_ternary() {
        let (output, count) = run_convert("cond ? doA() : doB();");
        assert!(count >= 1, "Should have converted at least 1 ternary, got: {count}");
        assert!(output.contains("if"), "Should contain 'if', got: {output}");
        assert!(output.contains("else"), "Should contain 'else', got: {output}");
        assert!(
            !output.contains('?'),
            "Should NOT contain '?' ternary operator, got: {output}"
        );
    }

    #[test]
    fn test_preserve_assignment_ternary() {
        let (output, count) = run_convert("var x = cond ? 1 : 2;");
        assert_eq!(
            count, 0,
            "Should NOT convert ternary inside var declaration, got: {count}"
        );
        assert!(
            output.contains('?'),
            "Should still contain '?' ternary operator, got: {output}"
        );
    }

    #[test]
    fn test_preserve_return_ternary() {
        let (output, count) = run_convert("function f() { return cond ? 1 : 2; }");
        assert_eq!(count, 0, "Should NOT convert ternary inside return, got: {count}");
        assert!(
            output.contains('?'),
            "Should still contain '?' ternary operator, got: {output}"
        );
    }

    #[test]
    fn test_convert_in_function_body() {
        let (output, count) = run_convert("function f() { cond ? doA() : doB(); }");
        assert!(
            count >= 1,
            "Should have converted ternary in function body, got: {count}"
        );
        assert!(output.contains("if"), "Should contain 'if', got: {output}");
    }

    #[test]
    fn test_convert_in_block() {
        let (output, count) = run_convert("{ cond ? doA() : doB(); }");
        assert!(count >= 1, "Should have converted ternary in block, got: {count}");
        assert!(output.contains("if"), "Should contain 'if', got: {output}");
    }

    #[test]
    fn test_preserve_arrow_expression_body() {
        let (output, count) = run_convert("const f = (K) => K ? 1 : 2;");
        assert_eq!(
            count, 0,
            "Should NOT convert ternary in arrow expression body, got: {count}"
        );
        assert!(
            output.contains('?'),
            "Should still contain '?' ternary operator, got: {output}"
        );
    }

    #[test]
    fn splits_return_of_nested_ternary() {
        let (output, count) = run_convert("function f() { return a ? b : c ? d : e; }");
        assert_eq!(count, 1);
        assert_eq!(
            output,
            "function f() {\n\tif (a) {\n\t\treturn b;\n\t}\n\treturn c ? d : e;\n}\n"
        );
    }

    #[test]
    fn splits_return_with_sequence_branch_unparenthesized() {
        let (output, count) = run_convert("function f() { return a ? (x(), y) : z; }");
        assert_eq!(count, 1);
        assert!(output.contains("return x(), y;"), "got: {output}");
        assert!(output.contains("return z;"), "got: {output}");
    }

    #[test]
    fn keeps_ternary_in_expression_bodied_arrow() {
        let (output, count) = run_convert("const f = () => a ? b : c ? d : e;");
        assert_eq!(count, 0, "got: {output}");
    }

    #[test]
    fn converts_in_switch_case() {
        let (output, count) = run_convert("switch (k) { case 1: a ? b() : c(); }");
        assert_eq!(count, 1);
        assert!(output.contains("if (a)"), "got: {output}");
    }

    #[test]
    fn converts_nested_ternaries_in_one_traversal() {
        let (output, count) =
            run_convert("a ? (b ? c() : d()) : e(); function f() { return a ? b : c ? d : e ? g : h; }");
        assert_eq!(count, 4, "got: {output}");
        assert!(!output.contains("b ?"), "got: {output}");
        assert!(output.contains("\treturn e ? g : h;"), "got: {output}");
    }
}
