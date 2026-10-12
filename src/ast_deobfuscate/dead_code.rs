use oxc_allocator::{TakeIn, Vec as OxcVec};
use oxc_ast::ast::{
    ArrowFunctionExpression, BindingPattern, BlockStatement, Class, Expression, Function, Ident, Statement,
    VariableDeclaration, VariableDeclarationKind, VariableDeclarator,
};
use oxc_ast_visit::Visit;
use oxc_semantic::ScopeFlags;
use oxc_span::SPAN;
use oxc_traverse::{Traverse, TraverseCtx};

use crate::ast_deobfuscate::pass::counter_pass;
use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

/// Removes branches and loops whose literal test can never select them, and
/// statements after an unconditional jump. Dropped code may still declare
/// hoisted bindings, so its `var` names are kept as a bare `var` statement and
/// unreachable function, class and lexical declarations are kept verbatim.
pub struct DeadCodeEliminator {
    rewrites: usize,
}

counter_pass!(DeadCodeEliminator, rewrites, rewrites);

/// `ToBoolean` of a literal test, or `None` when the test is not a literal.
fn literal_truthiness(expr: &Expression<'_>) -> Option<bool> {
    match expr.without_parentheses() {
        Expression::BooleanLiteral(lit) => Some(lit.value),
        Expression::NumericLiteral(lit) => Some(lit.value != 0.0 && !lit.value.is_nan()),
        Expression::StringLiteral(lit) => Some(!lit.value.is_empty()),
        Expression::NullLiteral(_) => Some(false),
        _ => None,
    }
}

/// `var` names declared by dropped code; nested functions and classes keep
/// their own declarations.
#[derive(Default)]
struct HoistedVars<'a> {
    names: Vec<Ident<'a>>,
    has_function_declaration: bool,
}

impl<'a> HoistedVars<'a> {
    fn into_statement(self, ctx: &Ctx<'a>) -> Option<Statement<'a>> {
        if self.names.is_empty() {
            return None;
        }
        let declarators = OxcVec::from_iter_in(
            self.names.into_iter().map(|name| {
                let id = BindingPattern::new_binding_identifier(SPAN, name, ctx);
                VariableDeclarator::new(SPAN, id, None, None, false, ctx)
            }),
            ctx,
        );
        Some(Statement::new_variable_declaration(
            SPAN,
            VariableDeclarationKind::Var,
            declarators,
            false,
            ctx,
        ))
    }
}

impl<'a> Visit<'a> for HoistedVars<'a> {
    fn visit_variable_declaration(&mut self, it: &VariableDeclaration<'a>) {
        if !it.kind.is_var() {
            return;
        }
        for declarator in &it.declarations {
            for binding in declarator.id.get_binding_identifiers() {
                if !self.names.contains(&binding.name) {
                    self.names.push(binding.name);
                }
            }
        }
    }

    fn visit_function(&mut self, it: &Function<'a>, _flags: ScopeFlags) {
        self.has_function_declaration |= it.is_declaration();
    }

    fn visit_arrow_function_expression(&mut self, _it: &ArrowFunctionExpression<'a>) {}

    fn visit_class(&mut self, _it: &Class<'a>) {}

    fn visit_expression(&mut self, _it: &Expression<'a>) {}
}

impl DeadCodeEliminator {
    /// Replacement for a statement whose live part is `kept` and whose dead
    /// part is `dropped`, or `None` when dropping is unsound.
    fn replace_branch<'a>(
        kept: Option<&mut Statement<'a>>,
        dropped: Option<&Statement<'a>>,
        ctx: &mut Ctx<'a>,
    ) -> Option<Statement<'a>> {
        let mut hoisted = HoistedVars::default();
        if let Some(dropped) = dropped {
            hoisted.visit_statement(dropped);
        }
        // Sloppy-mode (Annex B) block functions also create a `var` binding.
        if hoisted.has_function_declaration && !ctx.current_scope_flags().is_strict_mode() {
            return None;
        }
        let kept = kept.map(|stmt| stmt.take_in(ctx));
        let vars = hoisted.into_statement(ctx);
        Some(match (kept, vars) {
            (None, None) => Statement::new_empty_statement(SPAN, ctx),
            (Some(stmt), None) | (None, Some(stmt)) => stmt,
            (Some(kept), Some(vars)) => {
                let scope_id = ctx.insert_scope_below_statement(&kept, ScopeFlags::empty());
                Statement::new_block_statement_with_scope_id(SPAN, [kept, vars], scope_id, ctx)
            }
        })
    }

    const fn is_terminator(stmt: &Statement<'_>) -> bool {
        matches!(
            stmt,
            Statement::ReturnStatement(_)
                | Statement::BreakStatement(_)
                | Statement::ContinueStatement(_)
                | Statement::ThrowStatement(_)
        )
    }

    /// Declarations that bind a name even when never executed.
    fn is_kept_declaration(stmt: &Statement<'_>) -> bool {
        match stmt {
            Statement::FunctionDeclaration(_) | Statement::ClassDeclaration(_) => true,
            Statement::VariableDeclaration(decl) => !decl.kind.is_var(),
            _ => false,
        }
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for DeadCodeEliminator {
    fn exit_statement(&mut self, stmt: &mut Statement<'a>, ctx: &mut Ctx<'a>) {
        let replacement = match stmt {
            Statement::IfStatement(if_stmt) => {
                let Some(truthy) = literal_truthiness(&if_stmt.test) else {
                    return;
                };
                let if_stmt = &mut **if_stmt;
                let (kept, dropped) = if truthy {
                    (Some(&mut if_stmt.consequent), if_stmt.alternate.as_ref())
                } else {
                    (if_stmt.alternate.as_mut(), Some(&if_stmt.consequent))
                };
                // A bare function declaration as an `if` body is block scoped
                // there but would hoist once lifted out.
                if matches!(kept.as_deref(), Some(Statement::FunctionDeclaration(_))) {
                    return;
                }
                Self::replace_branch(kept, dropped, ctx)
            }
            Statement::WhileStatement(while_stmt) if literal_truthiness(&while_stmt.test) == Some(false) => {
                Self::replace_branch(None, Some(&while_stmt.body), ctx)
            }
            _ => return,
        };
        if let Some(replacement) = replacement {
            self.rewrites += 1;
            *stmt = replacement;
        }
    }

    fn exit_block_statement(&mut self, block: &mut BlockStatement<'a>, ctx: &mut Ctx<'a>) {
        let Some(terminator) = block.body.iter().position(Self::is_terminator) else {
            return;
        };
        if terminator + 1 == block.body.len() {
            return;
        }
        let mut hoisted = HoistedVars::default();
        for stmt in &block.body[terminator + 1..] {
            if !Self::is_kept_declaration(stmt) {
                hoisted.visit_statement(stmt);
            }
        }
        if hoisted.has_function_declaration && !ctx.current_scope_flags().is_strict_mode() {
            return;
        }
        let mut body = OxcVec::with_capacity_in(block.body.len(), ctx);
        for (index, stmt) in block.body.take_in(ctx).into_iter().enumerate() {
            if index <= terminator || Self::is_kept_declaration(&stmt) {
                body.push(stmt);
            }
        }
        body.extend(hoisted.into_statement(ctx));
        self.rewrites += 1;
        block.body = body;
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

    fn run_dce_with(code: &str, source_type: SourceType) -> String {
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, code, source_type).parse();
        assert!(ret.diagnostics.is_empty(), "{:?}", ret.diagnostics);
        let mut program = ret.program;

        let mut eliminator = DeadCodeEliminator::new();
        let scoping = semantic::scoping(&program);
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        traverse_mut_with_ctx(&mut eliminator, &mut program, &mut ctx);

        Codegen::new().build(&program).code
    }

    fn run_dce(code: &str) -> String {
        run_dce_with(code, SourceType::mjs())
    }

    fn normalized(code: &str) -> String {
        run_dce_with(code, SourceType::mjs())
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn if_false_without_else_is_removed() {
        assert_eq!(normalized("if (false) { console.log('dead'); } f();"), "; f();");
    }

    #[test]
    fn while_false_is_removed() {
        assert_eq!(normalized("while (false) { console.log('dead'); } f();"), "; f();");
    }

    #[test]
    fn reachable_code_is_preserved() {
        assert_eq!(normalized("var x = 1; console.log(x);"), "var x = 1; console.log(x);");
    }

    #[test]
    fn if_true_keeps_consequent() {
        assert_eq!(normalized("if (1) { y(); }"), "{ y(); }");
        assert_eq!(normalized("if ('a') y(); else z();"), "y();");
    }

    #[test]
    fn if_false_keeps_alternate() {
        assert_eq!(normalized("if (0) { y() } else { z() } w();"), "{ z(); } w();");
        assert_eq!(
            normalized("function g(){ if (false) { a() } else { b() } }"),
            "function g() { { b(); } }"
        );
    }

    #[test]
    fn falsy_literals_use_to_boolean() {
        assert_eq!(normalized("if ('') a(); else b();"), "b();");
        assert_eq!(normalized("if (null) a(); else b();"), "b();");
    }

    #[test]
    fn dropped_branch_keeps_hoisted_var() {
        assert_eq!(normalized("if (1) a(); else var a;"), "{ a(); var a; }");
        assert_eq!(
            normalized("function f() { if (0) var x, y; y = 1; return y; } f();"),
            "function f() { var x, y; y = 1; return y; } f();"
        );
        assert_eq!(
            normalized("if (xxx) foo; else if (0) var a, b; else if (0) var c, d; f(a, b, c, d);"),
            "if (xxx) foo; else { var c, d; var a, b; } f(a, b, c, d);"
        );
        assert_eq!(
            normalized("while (0) { for (var i in o) { var j = i; } } f(i, j);"),
            "var i, j; f(i, j);"
        );
    }

    #[test]
    fn nested_function_vars_are_not_hoisted() {
        assert_eq!(normalized("if (0) { g(function () { var z; }); }"), ";");
    }

    #[test]
    fn non_literal_tests_are_untouched() {
        assert_eq!(normalized("if (x) a(); else b();"), "if (x) a(); else b();");
        assert_eq!(normalized("if (NaN) a();"), "if (NaN) a();");
    }

    #[test]
    fn sloppy_block_function_in_dropped_branch_is_kept() {
        let output = run_dce_with("if (0) { function f() {} } g(f);", SourceType::cjs());
        assert!(output.contains("function f"), "Annex B binds `f`, got: {output}");
    }

    #[test]
    fn sloppy_block_function_in_dropped_alternate_keeps_if() {
        let output = run_dce_with("if (1) a(); else { function f() {} }", SourceType::cjs());
        assert_eq!(
            output.split_whitespace().collect::<Vec<_>>().join(" "),
            "if (1) a(); else { function f() {} }"
        );
    }

    #[test]
    fn strict_block_function_in_dropped_branch_is_removed() {
        assert_eq!(normalized("if (0) { function f() {} } g(f);"), "; g(f);");
    }

    #[test]
    fn unreachable_statements_are_removed() {
        assert_eq!(
            normalized("function f() { { return 1; g(); h(); } }"),
            "function f() { { return 1; } }"
        );
    }

    #[test]
    fn unreachable_declarations_keep_their_bindings() {
        assert_eq!(
            normalized(
                "function f() { { return h; var a = g(); function h() {} let k = 1; class C {} if (x) { var b; } } }"
            ),
            "function f() { { return h; function h() {} let k = 1; class C {} var a, b; } }"
        );
    }
}
