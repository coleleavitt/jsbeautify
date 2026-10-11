//! Flatten Bun's top-level CJS wrapper expression.

use oxc_allocator::{Allocator, TakeIn, Vec as ArenaVec};
use oxc_ast::ast::{BindingPattern, Directive, Expression, Function, Program, Statement};
use oxc_ast::builder::AstBuilder;

/// Replaces a Bun standalone CJS wrapper:
///
/// `(function(exports, require, module, __filename, __dirname) { ... })`
///
/// with the wrapper body statements at program top level.
#[must_use]
pub fn flatten_top_level_cjs_wrapper<'a>(program: &mut Program<'a>, allocator: &'a Allocator) -> usize {
    if program.body.len() != 1 {
        return 0;
    }

    let mut stmt = program.body.remove(0);
    let Some((directives, statements)) = take_cjs_wrapper_body(&mut stmt, allocator) else {
        program.body.push(stmt);
        return 0;
    };

    let statement_count = statements.len();
    if program.directives.is_empty() {
        program.directives = directives;
    } else {
        for directive in directives {
            program.directives.push(directive);
        }
    }
    program.body = statements;
    statement_count
}

fn take_cjs_wrapper_body<'a>(
    stmt: &mut Statement<'a>,
    allocator: &'a Allocator,
) -> Option<(ArenaVec<'a, Directive<'a>>, ArenaVec<'a, Statement<'a>>)> {
    let Statement::ExpressionStatement(expr_stmt) = stmt else {
        return None;
    };
    let func = function_from_expr_mut(&mut expr_stmt.expression)?;
    if !is_cjs_wrapper_function(func) {
        return None;
    }
    let body = func.body.as_mut()?;
    let directives = body.directives.take_in(&AstBuilder::new(allocator));
    let statements = body.statements.take_in(&AstBuilder::new(allocator));
    Some((directives, statements))
}

fn function_from_expr_mut<'a, 'b>(expr: &'b mut Expression<'a>) -> Option<&'b mut Function<'a>> {
    match expr {
        Expression::FunctionExpression(func) => Some(func.as_mut()),
        Expression::ParenthesizedExpression(paren) => function_from_expr_mut(&mut paren.expression),
        _ => None,
    }
}

fn is_cjs_wrapper_function(func: &Function<'_>) -> bool {
    let cjs_hits = func
        .params
        .items
        .iter()
        .filter_map(|param| match &param.pattern {
            BindingPattern::BindingIdentifier(binding) => Some(binding.name.as_str()),
            _ => None,
        })
        .filter(|name| matches!(*name, "exports" | "require" | "module" | "__filename" | "__dirname"))
        .count();
    cjs_hits >= 3
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_allocator::Allocator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    fn run(code: &str) -> String {
        let allocator = Allocator::default();
        let parsed = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let mut program = parsed.program;
        let flattened = flatten_top_level_cjs_wrapper(&mut program, &allocator);
        assert!(flattened > 0);
        Codegen::new().build(&program).code
    }

    fn flatten_count(code: &str) -> usize {
        let allocator = Allocator::default();
        let parsed = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let mut program = parsed.program;
        flatten_top_level_cjs_wrapper(&mut program, &allocator)
    }

    #[test]
    fn flattens_bare_bun_cjs_wrapper() {
        let out = run("(function(exports, require, module, __filename, __dirname) { var x = 1; x++; })");
        assert!(!out.contains("function(exports"), "wrapper should be removed: {out}");
        assert!(out.contains("var x = 1"), "body should remain: {out}");
    }

    #[test]
    fn preserves_use_strict_directive() {
        let out = run("(function(exports, require, module) { \"use strict\"; var x = 1; })");
        assert!(
            out.starts_with("\"use strict\";"),
            "directive should move to program: {out}"
        );
    }

    #[test]
    fn does_not_flatten_called_iife_without_argument_binding() {
        let flattened = flatten_count(
            "(function(exports, require, module) { use(exports); })(myExports, customRequire, myModule);",
        );
        assert_eq!(flattened, 0);
    }
}
