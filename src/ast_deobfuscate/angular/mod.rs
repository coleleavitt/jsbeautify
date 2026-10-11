//! Angular (Ivy) deobfuscation.
//!
//! Production Angular builds keep the Ivy definition keys (`ɵcmp`, `ɵdir`,
//! `ɵpipe`) and their metadata strings while minifying everything else:
//!
//! ```js
//! var Oc = class e {
//!     static ɵcmp = Vt({ type: e, selectors: [["app-login"]], consts: [...],
//!         template: function(n, r) { n & 1 && (q(0, "form", 0), _e(1, "Email"), Z()) } });
//! };
//! var EN = zf("[Auth] Login Success", hD());
//! ```
//!
//! - [`rename_angular_symbols`] renames the class bindings named by a
//!   definition's `type` (and NgRx `createAction` results) by `SymbolId`.
//! - [`annotate_angular_templates`] reconstructs an approximate HTML template
//!   from each component's instruction stream and inserts it as a comment
//!   above the `template` property of the final output.

mod collector;
mod naming;
mod rename;
mod template;

pub use rename::rename_angular_symbols;
pub use template::annotate_angular_templates;

/// Ivy definition kinds that carry a name-bearing `type` reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DefKind {
    Component,
    Directive,
    Pipe,
}

impl DefKind {
    fn from_key(key: &str) -> Option<Self> {
        match key {
            "ɵcmp" => Some(Self::Component),
            "ɵdir" => Some(Self::Directive),
            "ɵpipe" => Some(Self::Pipe),
            _ => None,
        }
    }

    const fn suffix(self) -> &'static str {
        match self {
            Self::Component => "Component",
            Self::Directive => "Directive",
            Self::Pipe => "Pipe",
        }
    }
}

#[cfg(test)]
mod tests;
