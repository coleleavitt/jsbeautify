//! Do-while-switch dispatcher detector.
//!
//! Finds dispatcher functions of the form
//! `var F = function (STATE, ARGS) { do { switch (STATE) { … } } while (STATE != SENTINEL); }`
//! and, for each one, the values its state can take: the case labels, the
//! values each case body can leave in the state, and the values its calls
//! pass. Every name is resolved through `oxc_semantic`, so a shadowing
//! binding never stands in for the state or a constant. The scan does not
//! change the AST; [`super::dowhile_switch_cleaner`] prunes what it finds
//! unreachable.
//!
//! Like the other passes, the analysis assumes the built-ins are unmodified:
//! `Function.prototype.call` calls its receiver, and arrays convert to
//! strings through the standard `toString`/`join` (no
//! `Symbol.toPrimitive`). It does not model non-standard reflection either:
//! a sloppy-mode `fn.caller` or `arguments.callee` that hands the
//! dispatcher to other code.

mod bindings;
mod calls;
mod detector;
mod extraction;
mod transition;
mod types;
mod value;
mod walker;

pub use detector::detect;
pub use types::{DoWhileCase, DoWhileDispatcher, DoWhileDispatcherMap};
pub use value::StateValue;

#[cfg(test)]
mod tests;
