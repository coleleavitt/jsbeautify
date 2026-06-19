//! 1-arg lookup forwarder inliner.
//!
//! Detects functions of the exact shape
//!
//! ```text
//! function bI(x) { return <accessor>()[x]; }
//! var bI = function(x) { return <accessor>()[x]; };
//! ```
//!
//! where `<accessor>` is a plain identifier (ideally a self-init accessor).
//! Every call site `bI(y)` is rewritten to `<accessor>()[y]` and the forwarder
//! declaration removed.
//!
//! Akamai BMP uses this idiom heavily: `bI`, `QI`, `fG`, `jU`, `hY`, `sK`, `mU`,
//! `qm`, `WW` — all tiny 1-arg forwarders to `x19()` / `Y49()` lookup tables.
//! After inlining, call sites become `x19()[index]` which reads as ordinary
//! array indexing.

use oxc_traverse::TraverseCtx;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

mod collector;
mod inliner;
mod types;

pub use collector::LookupForwarderCollector;
pub use inliner::LookupForwarderInliner;
pub use types::LookupForwarderInfo;

#[cfg(test)]
mod tests;
