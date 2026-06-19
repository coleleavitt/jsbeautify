//! TypeScript enum IIFE annotation + namespace module naming + dynamic import annotation
//!
//! Three post-codegen string-level annotation passes:
//!
//! 1. **Enum annotation**: Detects TypeScript-compiled enum IIFEs:
//!    `(function(H) { H.Foo = "bar"; ... })(X || (X = {}));`
//!    and adds `/* enum EnumName { Foo="bar", ... } */` above them.
//!
//! 2. **Namespace module naming**: For `__export(CjH, { name1: ..., name2: ... })`,
//!    derives a module name from the first export and annotates with a comment.
//!
//! 3. **Arrow sequence expansion**: Converts expression-body arrows containing
//!    sequence expressions into block bodies for readability.

mod arrow_sequence;
mod enums;
mod namespace;
mod syntax_scan;

pub use arrow_sequence::expand_arrow_sequences;
pub use enums::annotate_ts_enums;
pub use namespace::annotate_namespace_modules;

#[cfg(test)]
mod tests;
