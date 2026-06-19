//! AST-based statement matching using structure hashes.

mod identifiers;
mod normalizer;
#[cfg(test)]
mod tests;

use oxc_allocator::Allocator;
use oxc_ast_visit::Visit;
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType};
use rustc_hash::{FxHashMap, FxHashSet};

#[derive(Debug, Clone)]
pub struct IdentifierInfo {
    pub name: String,
    pub start: u32,
    pub end: u32,
    pub order_index: usize,
}

#[derive(Debug, Clone)]
pub struct StatementInfo {
    pub start: u32,
    pub end: u32,
    pub hash: String,
    pub identifiers: Vec<IdentifierInfo>,
}

pub struct StatementMatcher {
    max_depth: usize,
}

impl StatementMatcher {
    #[must_use]
    pub const fn new(max_depth: usize) -> Self {
        Self { max_depth }
    }

    #[must_use]
    pub fn extract_statements(&self, source: &str) -> Vec<StatementInfo> {
        let allocator = Allocator::default();
        let source_type = SourceType::mjs();

        let parse_result = Parser::new(&allocator, source, source_type).parse();

        if !parse_result.errors.is_empty() {
            eprintln!(
                "[AST_MATCHER] Parse had {} errors (continuing anyway), first: {:?}",
                parse_result.errors.len(),
                parse_result.errors.first()
            );
        }

        let mut statements = Vec::new();
        eprintln!(
            "[AST_MATCHER] Program body has {} statements",
            parse_result.program.body.len()
        );

        for stmt in &parse_result.program.body {
            let mut normalizer = normalizer::AstNormalizer::new(self.max_depth);
            normalizer.visit_statement(stmt);
            let hash = format!("{:x}", md5::compute(&normalizer.output));

            let mut id_collector = identifiers::IdentifierCollector::new();
            id_collector.visit_statement(stmt);

            statements.push(StatementInfo {
                start: stmt.span().start,
                end: stmt.span().end,
                hash,
                identifiers: id_collector.identifiers,
            });
        }

        statements
    }

    #[must_use]
    pub fn match_statements<'a>(
        &self,
        source_stmts: &'a [StatementInfo],
        target_stmts: &'a [StatementInfo],
    ) -> Vec<(&'a StatementInfo, &'a StatementInfo)> {
        let mut target_by_hash: FxHashMap<&str, Vec<&StatementInfo>> = FxHashMap::default();
        for stmt in target_stmts {
            target_by_hash.entry(&stmt.hash).or_default().push(stmt);
        }

        let mut used_targets: FxHashSet<(u32, u32)> = FxHashSet::default();
        let mut matches = Vec::new();

        for source_stmt in source_stmts {
            let Some(candidates) = target_by_hash.get(source_stmt.hash.as_str()) else {
                continue;
            };

            let target_stmt = candidates.iter().find(|t| !used_targets.contains(&(t.start, t.end)));

            if let Some(&target_stmt) = target_stmt {
                used_targets.insert((target_stmt.start, target_stmt.end));
                matches.push((source_stmt, target_stmt));
            }
        }

        matches
    }
}

impl Default for StatementMatcher {
    fn default() -> Self {
        Self::new(12)
    }
}
