use oxc_traverse::TraverseCtx;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

mod collector;
mod inliner;
mod types;

pub use collector::FunctionCollector;
pub use inliner::FunctionInliner;
pub use types::{BinaryOp, InlinableFunction, ReturnExpr, UnaryOp};

#[cfg(test)]
mod tests;
