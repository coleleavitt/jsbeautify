use oxc_traverse::TraverseCtx;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

mod collector;
mod eliminator;
mod types;

pub use collector::DeadVarCollector;
pub use eliminator::DeadVarEliminator;

#[cfg(test)]
mod tests;
