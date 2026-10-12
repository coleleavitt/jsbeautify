//! The one entry point for `oxc_semantic` analysis in the pipeline.
//!
//! The pipeline rebuilds semantic data for the same program many times. Without
//! `Stats`, every `SemanticBuilder` build first walks the whole AST only to
//! size its tables. Each build here records the stats of the program it
//! analysed, and the next build of the same program passes them back, so only
//! the first build of a program pays for that walk.
//!
//! The stats are a capacity hint: wrong stats cost a reallocation, never a
//! different result.

use std::cell::Cell;

use oxc_ast::ast::Program;
use oxc_semantic::{Scoping, SemanticBuilder, SemanticBuilderReturn, Stats};

/// Address and length of a program's source text. A program keeps its source
/// text for its whole life, and a re-parsed or nested program has another one.
type ProgramKey = (usize, usize);

thread_local! {
    static LAST_STATS: Cell<Option<(ProgramKey, Stats)>> = const { Cell::new(None) };
}

fn key_of(program: &Program<'_>) -> ProgramKey {
    (program.source_text.as_ptr() as usize, program.source_text.len())
}

/// Analyses `program` with a `SemanticBuilder` that `configure` set up.
pub fn build_with<'a>(
    program: &'a Program<'a>,
    configure: impl FnOnce(SemanticBuilder<'a>) -> SemanticBuilder<'a>,
) -> SemanticBuilderReturn<'a> {
    let key = key_of(program);
    let mut builder = configure(SemanticBuilder::new());
    if let Some((last_key, stats)) = LAST_STATS.get()
        && last_key == key
    {
        builder = builder.with_stats(stats);
    }
    let built = builder.build(program);
    LAST_STATS.set(Some((key, built.semantic.stats())));
    built
}

/// Analyses `program` with the default `SemanticBuilder` settings.
pub fn build<'a>(program: &'a Program<'a>) -> SemanticBuilderReturn<'a> {
    build_with(program, |builder| builder)
}

/// Analyses `program` and keeps only its scoping.
pub fn scoping<'a>(program: &'a Program<'a>) -> Scoping {
    build(program).semantic.into_scoping()
}

/// Lets `program`, re-parsed from the code of the program analysed last, start
/// from that program's stats.
pub fn carry_stats_to(program: &Program<'_>) {
    if let Some((_, stats)) = LAST_STATS.get() {
        LAST_STATS.set(Some((key_of(program), stats)));
    }
}

#[cfg(test)]
mod tests {
    use oxc_allocator::Allocator;
    use oxc_ast::ast::Statement;
    use oxc_parser::Parser;
    use oxc_semantic::SemanticBuilder;
    use oxc_span::SourceType;

    use super::{build, carry_stats_to, scoping};

    fn summary(scoping: &oxc_semantic::Scoping) -> (usize, usize, usize, Vec<String>) {
        (
            scoping.symbols_len(),
            scoping.scopes_len(),
            scoping.references_len(),
            scoping.symbol_names().map(str::to_string).collect(),
        )
    }

    #[test]
    fn stale_stats_give_the_same_analysis_as_a_fresh_builder() {
        let allocator = Allocator::default();
        let small = Parser::new(&allocator, "var a = 1;", SourceType::cjs()).parse().program;
        let mut grown = Parser::new(&allocator, "var a = 1;", SourceType::cjs()).parse().program;
        let extra = Parser::new(
            &allocator,
            "function f(x) { let y = x; return () => y + a; }",
            SourceType::cjs(),
        )
        .parse()
        .program;

        // Record the stats of the small program, then let `grown` start from them.
        build(&small);
        carry_stats_to(&grown);
        for statement in extra.body {
            grown.body.push(statement);
        }
        assert!(matches!(grown.body.last(), Some(Statement::FunctionDeclaration(_))));

        let hinted = scoping(&grown);
        let fresh = SemanticBuilder::new().build(&grown).semantic.into_scoping();
        assert_eq!(summary(&hinted), summary(&fresh));
    }
}
