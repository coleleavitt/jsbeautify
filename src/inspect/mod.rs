//! Command-line inspection of Oxc's parser, semantic analysis and control-flow
//! graph, using the same Oxc revision as the deobfuscation passes.

pub mod ast;
pub mod cfg;
pub mod scope;

use oxc_allocator::Allocator;
use oxc_ast::ast::Program;
use oxc_diagnostics::OxcDiagnostic;
use oxc_parser::Parser;
use oxc_span::SourceType;
use serde::Serialize;

use crate::ParseGoal;

#[derive(Debug, thiserror::Error)]
pub enum InspectError {
    #[error("parse failed\n{}", .0.join("\n"))]
    Parse(Vec<String>),
    #[error("no function named `{0}`")]
    FunctionNotFound(String),
}

/// An inspection result plus the rendered non-fatal diagnostics met on the way.
#[derive(Debug)]
pub struct Inspection<T> {
    pub report: T,
    pub diagnostics: Vec<String>,
}

/// JavaScript parsed with `goal`, as the deobfuscation pipeline parses it,
/// optionally as TypeScript with JSX.
const fn source_type(goal: ParseGoal, typescript: bool) -> SourceType {
    let source_type = goal.source_type();
    if typescript {
        source_type.with_typescript(true).with_jsx(true)
    } else {
        source_type
    }
}

fn parse<'a>(
    allocator: &'a Allocator,
    source: &'a str,
    goal: ParseGoal,
    typescript: bool,
) -> Result<(Program<'a>, Vec<String>), InspectError> {
    let ret = Parser::new(allocator, source, source_type(goal, typescript)).parse();
    let diagnostics = render(source, ret.diagnostics.into_vec());
    if ret.panicked {
        return Err(InspectError::Parse(diagnostics));
    }
    Ok((ret.program, diagnostics))
}

fn render(source: &str, diagnostics: Vec<OxcDiagnostic>) -> Vec<String> {
    diagnostics
        .into_iter()
        .map(|diagnostic| diagnostic.render_with_source_code(source.to_owned()))
        .collect()
}

/// 1-based line and 1-based column counted in Unicode characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Position {
    pub line: u32,
    pub column: u32,
}

/// Line/column lookup for a fixed set of byte offsets, resolved in one pass
/// over the source so single-line bundles stay linear.
struct Positions {
    offsets: Vec<u32>,
    positions: Vec<Position>,
}

impl Positions {
    fn new(source: &str, mut offsets: Vec<u32>) -> Self {
        offsets.sort_unstable();
        offsets.dedup();
        let mut positions = Vec::with_capacity(offsets.len());
        let mut chars = source.char_indices().peekable();
        let mut position = Position { line: 1, column: 1 };
        for &offset in &offsets {
            while let Some(&(index, ch)) = chars.peek() {
                if index >= offset as usize {
                    break;
                }
                chars.next();
                if ch == '\n' {
                    position = Position {
                        line: position.line + 1,
                        column: 1,
                    };
                } else {
                    position.column += 1;
                }
            }
            positions.push(position);
        }
        Self { offsets, positions }
    }

    fn get(&self, offset: u32) -> Position {
        let index = self
            .offsets
            .binary_search(&offset)
            .expect("offset registered with Positions::new");
        self.positions[index]
    }
}
