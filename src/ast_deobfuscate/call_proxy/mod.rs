use oxc_traverse::TraverseCtx;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

mod collector;
mod inliner;
mod types;

pub use collector::CallProxyCollector;
pub use inliner::CallProxyInliner;
pub use types::CallProxyInfo;

#[cfg(test)]
mod tests;
