use super::{InspectError, Inspection, Position, Positions, parse, render};
use crate::ParseGoal;
use oxc_allocator::Allocator;
use oxc_ast::AstKind;
use oxc_cfg::graph::visit::EdgeRef;
use oxc_cfg::{BlockNodeId, ControlFlowGraph, EdgeType};
use oxc_semantic::dot::{DebugDot, DebugDotContext};
use oxc_semantic::{AstNode, AstNodes, SemanticBuilder};
use oxc_span::GetSpan;
use rustc_hash::FxHashSet;
use serde::Serialize;
use std::borrow::Cow;
use std::fmt::Write as _;

#[derive(Debug, Serialize)]
pub struct ControlFlow {
    pub blocks: Vec<Block>,
    pub edges: Vec<Edge>,
}

#[derive(Debug, Serialize)]
pub struct Block {
    pub id: usize,
    pub unreachable: bool,
    pub instructions: Vec<Instruction>,
}

#[derive(Debug, Serialize)]
pub struct Instruction {
    /// Oxc's instruction label, e.g. `VariableDeclaration`, `Condition(BinaryExpression)`, `return <value>`.
    pub label: String,
    pub position: Option<Position>,
}

#[derive(Debug, Serialize)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    /// Oxc's `EdgeType`, e.g. `Normal`, `Jump`, `Backedge`, `Error(Implicit)`.
    pub kind: String,
}

/// The control-flow graph `oxc_semantic` builds for the program.
///
/// With `function`, only the functions with that name are kept: by their own
/// name, or the variable, property, method or assignment target they are
/// bound to. Functions nested inside them are left out.
///
/// # Errors
/// Returns [`InspectError::Parse`] when the parser cannot recover, and
/// [`InspectError::FunctionNotFound`] when no function has that name.
///
/// # Panics
/// Panics only if `oxc_semantic` returns no CFG despite `with_cfg(true)`.
pub fn control_flow(
    source: &str,
    goal: ParseGoal,
    typescript: bool,
    function: Option<&str>,
) -> Result<Inspection<ControlFlow>, InspectError> {
    crate::stack::run_on_sized_stack(source.len(), || {
        control_flow_on_current_stack(source, goal, typescript, function)
    })
}

fn control_flow_on_current_stack(
    source: &str,
    goal: ParseGoal,
    typescript: bool,
    function: Option<&str>,
) -> Result<Inspection<ControlFlow>, InspectError> {
    let allocator = Allocator::default();
    let (program, mut diagnostics) = parse(&allocator, source, goal, typescript)?;
    let ret = SemanticBuilder::new()
        .with_build_nodes(true)
        .with_check_syntax_error(true)
        .with_cfg(true)
        .build(&program);
    diagnostics.extend(render(source, ret.diagnostics.into_vec()));
    let nodes = ret.semantic.nodes();
    let cfg = ret.semantic.cfg().expect("SemanticBuilder::with_cfg builds a CFG");

    let included: Option<FxHashSet<BlockNodeId>> = match function {
        Some(name) => {
            let entries: Vec<_> = nodes
                .iter()
                .filter(|node| function_name(nodes, node).is_some_and(|found| found == name))
                .map(|node| nodes.cfg_id(node.id()))
                .collect();
            if entries.is_empty() {
                return Err(InspectError::FunctionNotFound(name.to_owned()));
            }
            Some(function_blocks(cfg, entries))
        }
        None => None,
    };
    let is_included = |block: BlockNodeId| included.as_ref().is_none_or(|set| set.contains(&block));

    let offsets = cfg
        .graph
        .node_indices()
        .filter(|&block| is_included(block))
        .flat_map(|block| cfg.basic_block(block).instructions())
        .filter_map(|instruction| instruction.node_id)
        .map(|node_id| nodes.kind(node_id).span().start)
        .collect();
    let positions = Positions::new(source, offsets);
    let context = DebugDotContext::new(nodes, false);

    let mut blocks: Vec<_> = cfg
        .graph
        .node_indices()
        .filter(|&block| is_included(block))
        .map(|block| {
            let basic_block = cfg.basic_block(block);
            Block {
                id: cfg.graph[block].index(),
                unreachable: basic_block.is_unreachable(),
                instructions: basic_block
                    .instructions()
                    .iter()
                    .map(|instruction| Instruction {
                        label: instruction.debug_dot(context),
                        position: instruction
                            .node_id
                            .map(|node_id| positions.get(nodes.kind(node_id).span().start)),
                    })
                    .collect(),
            }
        })
        .collect();
    blocks.sort_by_key(|block| block.id);
    let mut edges: Vec<_> = cfg
        .graph
        .edge_references()
        .filter(|edge| is_included(edge.source()) && is_included(edge.target()))
        .map(|edge| Edge {
            from: cfg.graph[edge.source()].index(),
            to: cfg.graph[edge.target()].index(),
            kind: format!("{:?}", edge.weight()),
        })
        .collect();
    edges.sort_by_key(|edge| (edge.from, edge.to));
    Ok(Inspection {
        report: ControlFlow { blocks, edges },
        diagnostics,
    })
}

/// Blocks reachable from the function entries without entering nested functions.
fn function_blocks(cfg: &ControlFlowGraph, mut stack: Vec<BlockNodeId>) -> FxHashSet<BlockNodeId> {
    let mut seen: FxHashSet<_> = stack.iter().copied().collect();
    while let Some(block) = stack.pop() {
        for edge in cfg.graph.edges(block) {
            if !matches!(edge.weight(), EdgeType::NewFunction) && seen.insert(edge.target()) {
                stack.push(edge.target());
            }
        }
    }
    seen
}

fn function_name<'a>(nodes: &AstNodes<'a>, node: &AstNode<'a>) -> Option<Cow<'a, str>> {
    match node.kind() {
        AstKind::Function(function) => {
            if let Some(id) = &function.id {
                return Some(Cow::Borrowed(id.name.as_str()));
            }
        }
        AstKind::ArrowFunctionExpression(_) => {}
        _ => return None,
    }
    match nodes.parent_kind(node.id()) {
        AstKind::VariableDeclarator(declarator) => declarator
            .id
            .get_binding_identifier()
            .map(|id| Cow::Borrowed(id.name.as_str())),
        AstKind::MethodDefinition(method) => method.key.static_name(),
        AstKind::PropertyDefinition(property) => property.key.static_name(),
        AstKind::ObjectProperty(property) => property.key.static_name(),
        AstKind::AssignmentExpression(assignment) => assignment.left.get_identifier_name().map(Cow::Borrowed),
        _ => None,
    }
}

impl ControlFlow {
    /// Graphviz DOT, one box per basic block labelled with its instructions.
    #[must_use]
    pub fn to_dot(&self) -> String {
        let mut dot = String::from("digraph cfg {\n    node [shape=box, fontname=monospace];\n");
        for block in &self.blocks {
            let mut label = format!("bb{}\\l", block.id);
            for instruction in &block.instructions {
                label.push_str(&escape(&instruction.label));
                if let Some(Position { line, column }) = instruction.position {
                    let _ = write!(label, " @{line}:{column}");
                }
                label.push_str("\\l");
            }
            let style = if block.unreachable { ", style=dotted" } else { "" };
            let _ = writeln!(dot, "    bb{} [label=\"{label}\"{style}];", block.id);
        }
        for edge in &self.edges {
            let _ = writeln!(
                dot,
                "    bb{} -> bb{} [label=\"{}\"];",
                edge.from,
                edge.to,
                escape(&edge.kind)
            );
        }
        dot.push_str("}\n");
        dot
    }
}

fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\l")
}
