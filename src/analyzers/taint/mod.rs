//! Taint-lite: source → sink reachability for client-side bug finding.
//!
//! The other analyzers find *where* a dangerous sink is (`el.innerHTML = x`).
//! This one asks the question that turns a sink into a *bug*: does
//! attacker-controlled input actually reach it? It is the first, deliberately
//! small, step toward the ROADMAP P0 taint engine — an intra-procedural,
//! flow-insensitive approximation grounded in oxc's resolved symbols.
//!
//! Algorithm (per program, symbol-resolved via `oxc_semantic`):
//!  1. **Seed**: any binding initialized from a taint *source* expression
//!     (`location.hash`, `document.URL`, `location.search`, `window.name`,
//!     a `message`-event `.data`, etc.) marks that symbol tainted. Direct
//!     `source` sub-expressions are tainted too.
//!  2. **Propagate** (one fixpoint pass): `b = taintedExpr` taints `b`;
//!     string concatenation / template literals containing a tainted symbol
//!     stay tainted.
//!  3. **Report**: a *sink* (`innerHTML`/`outerHTML`/`document.write`/`eval`/
//!     `location.href = …`) whose argument expression contains a tainted
//!     symbol — or a taint source directly — produces a finding:
//!     `dom_xss` for HTML/eval sinks, `open_redirect` for location sinks.
//!
//! This is an over-approximation (flow-insensitive, no sanitizer modeling), so
//! findings are leads to verify, not proofs — but unlike the pure sink scan,
//! every finding has an attacker-controlled-data justification.

mod expr;
mod propagation;
mod reporter;
#[cfg(test)]
mod tests;

use std::collections::HashSet;

use oxc_allocator::Allocator;
use oxc_ast_visit::Visit;
use oxc_parser::Parser;
use oxc_span::SourceType;

use super::match_kind::{AnalyzerMatch, FileType};
use propagation::{CallPropagator, FunctionParams, TaintCollector};
use reporter::SinkReporter;

/// Run the taint-lite analysis on `source`.
#[must_use]
pub fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    let allocator = Allocator::default();
    let parse = Parser::new(&allocator, source, SourceType::mjs().with_jsx(true)).parse();

    let mut params = FunctionParams::default();
    params.visit_program(&parse.program);

    let mut tainted: HashSet<String> = HashSet::new();
    for _ in 0..8 {
        let before = tainted.len();
        let mut collector = TaintCollector { tainted: &mut tainted };
        collector.visit_program(&parse.program);
        let mut propagator = CallPropagator {
            params: &params.map,
            tainted: &mut tainted,
        };
        propagator.visit_program(&parse.program);
        if tainted.len() == before {
            break;
        }
    }

    let mut reporter = SinkReporter {
        source,
        file_path,
        file_type,
        tainted: &tainted,
        out: Vec::new(),
    };
    reporter.visit_program(&parse.program);
    reporter.out
}
