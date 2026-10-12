//! Extents of the modules scope hoisting merged into one factory.
//!
//! `merge_modules` (`turbopack-ecmascript/src/lib.rs`) emits every merged
//! module as `// MERGED MODULE: <ident>` (an empty statement), its hoisted
//! statements, its body and its late statements, and inlines a dependency
//! where its hoisted `__turbopack_merged_esm__(i)` placeholder stood:
//!
//! - hoisted: `var __TURBOPACK__imported__module__<id>__ = ctx.i(id)` imports
//!   (a second importer of the same module gets its own alias variable),
//!   inlined dependencies, `ctx.S([…])` re-exports, `import.meta` and async
//!   dependency declarations;
//! - body: the module's statements, where each removed `import` / `export …
//!   from` declaration left an empty statement `;`;
//! - late: `ctx.s([…], id)`, the module's exports.
//!
//! There is no end marker, so a module's extent is inferred:
//!
//! 1. Once a module's body started, a later hoisted statement or marker
//!    belongs to an ancestor whose body has not started, so the modules in
//!    between have ended.
//! 2. A module's import variables are read only by that module's code, so a
//!    statement reading one belongs to it.
//! 3. A module's `ctx.s([…], id)` is its last statement, unless it directly
//!    follows the marker (a circuit breaker module exports first).
//! 4. An importer's body starts with the `;` of its removed imports: at
//!    least one per hoisted statement it has. A run of `;` after a module's
//!    code that is that long, and is followed by more code, starts the
//!    importer's body.
//!
//! On the unminified scope-hoisted Next.js 16.4 build these rules agree with
//! the source maps for about 99% of the statements; a module whose body is
//! only `;` followed by its importer's `;` cannot be told apart.

use oxc_ast::ast::{BindingPattern, Expression, IdentifierReference, Statement, VariableDeclaration};
use oxc_ast_visit::Visit;
use oxc_semantic::Scoping;
use oxc_span::GetSpan;
use oxc_syntax::symbol::SymbolId;
use rustc_hash::FxHashSet;
use serde::Serialize;

/// A module scope hoisting merged into a factory, from its
/// `// MERGED MODULE: <ident>` marker.
#[derive(Debug, Clone, Serialize)]
pub struct MergedModule {
    pub ident: String,
    /// From the marker to the end of its last statement, including the
    /// dependencies inlined into it.
    pub start: usize,
    pub end: usize,
    /// Index of the importer it is inlined into, in the factory's `merged` list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<usize>,
    /// Byte ranges of the module's own statements, without its inlined
    /// dependencies.
    pub own: Vec<(usize, usize)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Marker(usize),
    Import(SymbolId),
    Hoisted,
    /// `ctx.s([…], id?)`.
    Exports,
    Empty,
    /// A string expression statement such as `'use client'`.
    Directive,
    Code,
}

struct Frame {
    /// Statements of its hoisted part.
    hoisted: usize,
    imports: FxHashSet<SymbolId>,
}

/// The merged modules of a factory body with `markers` (comment start,
/// ident; sorted) and context parameter `context`.
pub(super) fn merged_modules(
    statements: &[Statement<'_>],
    body_start: usize,
    markers: &[(usize, String)],
    scoping: &Scoping,
    context: Option<SymbolId>,
) -> Vec<MergedModule> {
    let mut kinds = Vec::with_capacity(statements.len());
    let mut previous_end = body_start;
    for statement in statements {
        let kind = match classify(statement, previous_end, markers, scoping, context) {
            // A circuit breaker module exports first, so that a cyclic importer can
            // read its bindings (`references/esm/export.rs`).
            Kind::Exports if matches!(kinds.last(), Some(Kind::Marker(_))) => Kind::Hoisted,
            kind => kind,
        };
        kinds.push(kind);
        previous_end = statement.span().end as usize;
    }
    let mut frames: Vec<Frame> = Vec::new();
    let mut modules: Vec<MergedModule> = Vec::new();
    let mut owner: Vec<Option<usize>> = vec![None; statements.len()];
    let mut stack: Vec<usize> = Vec::new();
    let mut run: Vec<usize> = Vec::new();
    let reads = |index: usize| read_symbols(&statements[index], scoping);
    for (index, &kind) in kinds.iter().enumerate() {
        match kind {
            Kind::Marker(marker) => {
                split_run(&mut run, &mut stack, &frames, &kinds, &mut owner, &reads);
                if let Some(&top) = stack.last() {
                    frames[top].hoisted += 1;
                }
                let (start, ident) = &markers[marker];
                modules.push(MergedModule {
                    ident: ident.clone(),
                    start: *start,
                    end: statements[index].span().end as usize,
                    parent: stack.last().copied(),
                    own: Vec::new(),
                });
                frames.push(Frame {
                    hoisted: 0,
                    imports: FxHashSet::default(),
                });
                stack.push(frames.len() - 1);
                owner[index] = stack.last().copied();
            }
            Kind::Import(_) | Kind::Hoisted => {
                split_run(&mut run, &mut stack, &frames, &kinds, &mut owner, &reads);
                if let Some(&top) = stack.last() {
                    frames[top].hoisted += 1;
                    if let Kind::Import(symbol) = kind {
                        frames[top].imports.insert(symbol);
                    }
                    owner[index] = Some(top);
                }
            }
            _ if !stack.is_empty() => run.push(index),
            _ => {}
        }
    }
    split_run(&mut run, &mut stack, &frames, &kinds, &mut owner, &reads);

    for (index, statement) in statements.iter().enumerate() {
        let Some(module) = owner[index] else { continue };
        let span = statement.span();
        let start = if matches!(kinds[index], Kind::Marker(_)) {
            modules[module].start
        } else {
            span.start as usize
        };
        let own = &mut modules[module].own;
        match own.last_mut() {
            Some(last) if index > 0 && owner[index - 1] == Some(module) => last.1 = span.end as usize,
            _ => own.push((start, span.end as usize)),
        }
        let mut ancestor = Some(module);
        while let Some(m) = ancestor {
            modules[m].end = modules[m].end.max(span.end as usize);
            ancestor = modules[m].parent;
        }
    }
    modules
}

fn classify(
    statement: &Statement<'_>,
    previous_end: usize,
    markers: &[(usize, String)],
    scoping: &Scoping,
    context: Option<SymbolId>,
) -> Kind {
    match statement {
        Statement::EmptyStatement(empty) => {
            let first = markers.partition_point(|(start, _)| *start < previous_end);
            markers
                .get(first)
                .filter(|(start, _)| *start < empty.span.start as usize)
                .map_or(Kind::Empty, |_| {
                    // The marker closest to the statement.
                    let last = markers.partition_point(|(start, _)| *start < empty.span.start as usize);
                    Kind::Marker(last - 1)
                })
        }
        Statement::VariableDeclaration(decl) => hoisted_declaration(decl).unwrap_or(Kind::Code),
        Statement::ExpressionStatement(stmt) => match stmt.expression.without_parentheses() {
            Expression::StringLiteral(_) => Kind::Directive,
            Expression::CallExpression(call) => match context_member(&call.callee, scoping, context) {
                Some("s") => Kind::Exports,
                Some("S") => Kind::Hoisted,
                _ => Kind::Code,
            },
            _ => Kind::Code,
        },
        _ => Kind::Code,
    }
}

/// `var __TURBOPACK__imported__module__<id>__ = …` and the other declarations
/// Turbopack hoists (`import.meta`, async dependencies).
fn hoisted_declaration(decl: &VariableDeclaration<'_>) -> Option<Kind> {
    let [declarator] = decl.declarations.as_slice() else {
        return None;
    };
    let BindingPattern::BindingIdentifier(id) = &declarator.id else {
        return None;
    };
    if id.name.starts_with("__TURBOPACK__imported__module__") {
        id.symbol_id.get().map(Kind::Import)
    } else {
        matches!(
            id.name.as_str(),
            "__TURBOPACK__import$2e$meta__" | "__turbopack_async_dependencies__"
        )
        .then_some(Kind::Hoisted)
    }
}

fn context_member<'b>(callee: &'b Expression<'_>, scoping: &Scoping, context: Option<SymbolId>) -> Option<&'b str> {
    let Expression::StaticMemberExpression(member) = callee.without_parentheses() else {
        return None;
    };
    let Expression::Identifier(object) = member.object.without_parentheses() else {
        return None;
    };
    let symbol = scoping.get_reference(object.reference_id()).symbol_id();
    (symbol.is_some() && symbol == context).then_some(member.property.name.as_str())
}

/// The symbols `statement` reads.
fn read_symbols(statement: &Statement<'_>, scoping: &Scoping) -> FxHashSet<SymbolId> {
    struct Reads<'s> {
        scoping: &'s Scoping,
        symbols: FxHashSet<SymbolId>,
    }
    impl<'a> Visit<'a> for Reads<'_> {
        fn visit_identifier_reference(&mut self, id: &IdentifierReference<'a>) {
            if let Some(symbol) = self.scoping.get_reference(id.reference_id()).symbol_id() {
                self.symbols.insert(symbol);
            }
        }
    }
    let mut reads = Reads {
        scoping,
        symbols: FxHashSet::default(),
    };
    reads.visit_statement(statement);
    reads.symbols
}

/// Assign a run of body statements to the open modules: the innermost one
/// first, then outward (see the module docs), and close every module that
/// received one.
fn split_run(
    run: &mut Vec<usize>,
    stack: &mut Vec<usize>,
    frames: &[Frame],
    kinds: &[Kind],
    owner: &mut [Option<usize>],
    reads: &impl Fn(usize) -> FxHashSet<SymbolId>,
) {
    if run.is_empty() {
        return;
    }
    // Open modules, innermost first.
    let chain: Vec<usize> = stack.iter().rev().copied().collect();
    let required: Vec<Option<usize>> = run
        .iter()
        .map(|&index| {
            if !matches!(kinds[index], Kind::Code | Kind::Exports) {
                return None;
            }
            let read = reads(index);
            (0..chain.len())
                .rev()
                .find(|&level| frames[chain[level]].imports.iter().any(|s| read.contains(s)))
        })
        .collect();
    let mut lowest_later = vec![usize::MAX; run.len() + 1];
    for r in (0..run.len()).rev() {
        lowest_later[r] = lowest_later[r + 1].min(required[r].unwrap_or(usize::MAX));
    }
    let blank = |kind: Kind| matches!(kind, Kind::Empty | Kind::Directive);
    let mut levels = vec![0; run.len()];
    let (mut level, mut seen_code, mut previous) = (0, false, None);
    let mut candidates: Vec<usize> = Vec::new();
    for (r, &index) in run.iter().enumerate() {
        let kind = kinds[index];
        if blank(kind) && seen_code && !previous.is_some_and(blank) {
            let directives = run[r..].iter().take_while(|&&i| kinds[i] == Kind::Directive).count();
            let empties = run[r + directives..]
                .iter()
                .take_while(|&&i| kinds[i] == Kind::Empty)
                .count();
            let next = run.get(r + directives + empties).map(|&i| kinds[i]);
            if next.is_some_and(|k| k != Kind::Exports) {
                candidates.push(r);
                if level + 1 < chain.len() && empties >= frames[chain[level + 1]].hoisted && lowest_later[r] > level {
                    level += 1;
                    seen_code = false;
                    candidates.clear();
                }
            }
        }
        if let Some(required) = required[r].filter(|&required| required > level) {
            let boundaries = &candidates[candidates.len().saturating_sub(required - level)..];
            for (b, &from) in boundaries.iter().enumerate() {
                let assigned = required - (boundaries.len() - 1 - b);
                levels[from..r].fill(assigned);
            }
            level = required;
            candidates.clear();
        }
        levels[r] = level;
        match kind {
            Kind::Code => seen_code = true,
            Kind::Exports if level + 1 < chain.len() => {
                level += 1;
                seen_code = false;
                candidates.clear();
            }
            _ => {}
        }
        previous = Some(kind);
    }
    for (&index, &l) in run.iter().zip(&levels) {
        owner[index] = Some(chain[l]);
    }
    let closed = levels.iter().max().copied().unwrap_or(0);
    stack.truncate(stack.len() - closed - 1);
    run.clear();
}
