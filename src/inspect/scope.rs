use super::{InspectError, Inspection, Position, Positions, parse, render};
use oxc_allocator::Allocator;
use oxc_ast::AstKind;
use oxc_semantic::{AstNodes, Reference, ScopeId, Scoping, SemanticBuilder, SymbolFlags, SymbolId};
use oxc_span::GetSpan;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ScopeReport {
    pub scopes: Vec<ScopeEntry>,
    pub symbols: Vec<SymbolEntry>,
    pub unresolved: Vec<UnresolvedEntry>,
}

#[derive(Debug, Serialize)]
pub struct ScopeEntry {
    pub id: usize,
    pub parent: Option<usize>,
    /// AST node that owns the scope, e.g. `Program`, `Function`, `BlockStatement`.
    pub kind: String,
    pub flags: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct SymbolEntry {
    pub id: usize,
    pub name: String,
    /// One of `var`, `let`, `const`, `param`, `function`, `class`, `catch`,
    /// `import`, `enum`, `type`.
    pub kind: &'static str,
    pub flags: Vec<&'static str>,
    pub scope: usize,
    pub scope_kind: String,
    #[serde(flatten)]
    pub position: Position,
    pub references: Vec<ReferenceEntry>,
}

#[derive(Debug, Serialize)]
pub struct ReferenceEntry {
    #[serde(flatten)]
    pub position: Position,
    /// `read`, `write`, `read_write`, or `type`.
    pub access: &'static str,
}

#[derive(Debug, Serialize)]
pub struct UnresolvedEntry {
    pub name: String,
    pub count: usize,
    pub references: Vec<ReferenceEntry>,
}

/// The `oxc_semantic` symbol table: every scope, every symbol with its
/// resolved references, and the unresolved (global) references.
///
/// # Errors
/// Returns [`InspectError::Parse`] when the parser cannot recover.
pub fn symbol_table(source: &str, typescript: bool) -> Result<Inspection<ScopeReport>, InspectError> {
    let allocator = Allocator::default();
    let (program, mut diagnostics) = parse(&allocator, source, typescript)?;
    let ret = SemanticBuilder::new()
        .with_build_nodes(true)
        .with_check_syntax_error(true)
        .build(&program);
    diagnostics.extend(render(source, ret.diagnostics.into_vec()));
    let scoping = ret.semantic.scoping();
    let nodes = ret.semantic.nodes();

    let mut unresolved: Vec<_> = scoping
        .root_unresolved_references()
        .iter()
        .map(|(name, ids)| (name.as_str(), ids))
        .collect();
    unresolved.sort_unstable_by_key(|&(name, _)| name);

    let offsets = scoping
        .symbol_ids()
        .map(|symbol| scoping.symbol_span(symbol).start)
        .chain(
            scoping
                .symbol_ids()
                .flat_map(|symbol| scoping.get_resolved_references(symbol))
                .map(|reference| reference_start(nodes, reference)),
        )
        .chain(
            unresolved
                .iter()
                .flat_map(|(_, ids)| ids.iter())
                .map(|&id| reference_start(nodes, scoping.get_reference(id))),
        )
        .collect();
    let positions = Positions::new(source, offsets);

    let scopes = (0..scoping.scopes_len())
        .map(ScopeId::from_usize)
        .map(|scope| ScopeEntry {
            id: scope.index(),
            parent: scoping.scope_parent_id(scope).map(ScopeId::index),
            kind: scope_kind(scoping, nodes, scope),
            flags: scoping.scope_flags(scope).iter_names().map(|(name, _)| name).collect(),
        })
        .collect();
    let symbols = scoping
        .symbol_ids()
        .map(|symbol| {
            let scope = scoping.symbol_scope_id(symbol);
            SymbolEntry {
                id: symbol.index(),
                name: scoping.symbol_name(symbol).to_owned(),
                kind: symbol_kind(scoping, nodes, symbol),
                flags: scoping
                    .symbol_flags(symbol)
                    .iter_names()
                    .map(|(name, _)| name)
                    .collect(),
                scope: scope.index(),
                scope_kind: scope_kind(scoping, nodes, scope),
                position: positions.get(scoping.symbol_span(symbol).start),
                references: reference_entries(scoping.get_resolved_references(symbol), nodes, &positions),
            }
        })
        .collect();
    let unresolved = unresolved
        .into_iter()
        .map(|(name, ids)| UnresolvedEntry {
            name: name.to_owned(),
            count: ids.len(),
            references: reference_entries(ids.iter().map(|&id| scoping.get_reference(id)), nodes, &positions),
        })
        .collect();

    Ok(Inspection {
        report: ScopeReport {
            scopes,
            symbols,
            unresolved,
        },
        diagnostics,
    })
}

fn reference_start(nodes: &AstNodes<'_>, reference: &Reference) -> u32 {
    nodes.kind(reference.node_id()).span().start
}

fn reference_entries<'r>(
    references: impl Iterator<Item = &'r Reference>,
    nodes: &AstNodes<'_>,
    positions: &Positions,
) -> Vec<ReferenceEntry> {
    let mut entries: Vec<_> = references
        .map(|reference| {
            let start = reference_start(nodes, reference);
            (
                start,
                ReferenceEntry {
                    position: positions.get(start),
                    access: access(reference),
                },
            )
        })
        .collect();
    entries.sort_by_key(|&(start, _)| start);
    entries.into_iter().map(|(_, entry)| entry).collect()
}

fn scope_kind(scoping: &Scoping, nodes: &AstNodes<'_>, scope: ScopeId) -> String {
    format!("{:?}", nodes.kind(scoping.get_node_id(scope)).ty())
}

fn symbol_kind(scoping: &Scoping, nodes: &AstNodes<'_>, symbol: SymbolId) -> &'static str {
    let flags = scoping.symbol_flags(symbol);
    if flags.intersects(SymbolFlags::Import | SymbolFlags::TypeImport) {
        "import"
    } else if flags.contains(SymbolFlags::Function) {
        "function"
    } else if flags.contains(SymbolFlags::Class) {
        "class"
    } else if flags.contains(SymbolFlags::CatchVariable) {
        "catch"
    } else if flags.contains(SymbolFlags::ConstVariable) {
        "const"
    } else if flags.contains(SymbolFlags::BlockScopedVariable) {
        "let"
    } else if flags.contains(SymbolFlags::FunctionScopedVariable) {
        if matches!(
            nodes.kind(scoping.symbol_declaration(symbol)),
            AstKind::FormalParameter(_)
        ) {
            "param"
        } else {
            "var"
        }
    } else if flags.intersects(SymbolFlags::Enum) {
        "enum"
    } else {
        "type"
    }
}

fn access(reference: &Reference) -> &'static str {
    match (reference.flags().is_read(), reference.flags().is_write()) {
        (true, true) => "read_write",
        (true, false) => "read",
        (false, true) => "write",
        (false, false) => "type",
    }
}
