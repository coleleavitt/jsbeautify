use oxc_traverse::TraverseCtx;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

mod collector;
mod eliminator;
mod sweep;
mod types;

pub use collector::DeadVarCollector;
pub use eliminator::DeadVarEliminator;
pub use sweep::{DeadVarSweepResult, eliminate_dead_vars};

#[cfg(test)]
mod tests;
