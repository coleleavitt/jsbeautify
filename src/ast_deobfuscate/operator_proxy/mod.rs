use oxc_traverse::TraverseCtx;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

mod collector;
mod inliner;
mod types;

pub use collector::OperatorProxyCollector;
pub use inliner::OperatorProxyInliner;
pub use types::OperatorProxyInfo;

#[cfg(test)]
mod tests;
