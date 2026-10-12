use oxc_ast::ast::Expression;
use oxc_semantic::Scoping;
use oxc_traverse::Ancestor;

use super::Ctx;

/// The global object an alias holds, which decides the names that are safe
/// to read bare through it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobalObject {
    /// `window`: a browser window.
    Window,
    /// `self`: a browser window or any kind of worker.
    WindowOrWorker,
    /// `global`: Node.
    Node,
    /// `globalThis`, or an alias assigned more than one kind of global.
    Any,
}

impl GlobalObject {
    /// The global object both `self` and `other` may be.
    pub(super) fn merge(self, other: Self) -> Self {
        match (self, other) {
            _ if self == other => self,
            (Self::Window | Self::WindowOrWorker, Self::Window | Self::WindowOrWorker) => Self::WindowOrWorker,
            _ => Self::Any,
        }
    }
}

pub(super) fn global_object(expr: &Expression<'_>, scoping: &Scoping) -> Option<GlobalObject> {
    // NOTE: `this` is deliberately NOT global-like. `var self = this; self.foo`
    // inside a method/class reads an instance property — rewriting it to bare
    // `foo` silently turns it into a global lookup (a real miscompile). Only the
    // genuine global objects qualify, read through an unbound name: a local or
    // parameter called `window` is an ordinary value.
    let Expression::Identifier(id) = expr else {
        return None;
    };
    let unbound = id
        .reference_id
        .get()
        .is_some_and(|r| scoping.get_reference(r).symbol_id().is_none());
    if !unbound {
        return None;
    }
    match id.name.as_str() {
        "window" => Some(GlobalObject::Window),
        "self" => Some(GlobalObject::WindowOrWorker),
        "global" => Some(GlobalObject::Node),
        "globalThis" => Some(GlobalObject::Any),
        _ => None,
    }
}

/// A reserved word can never be emitted as a bare identifier, so rewriting
/// `alias.in` → `in` would produce invalid JavaScript. The pipeline always
/// parses as `SourceType::mjs()` (ES module = always strict mode), so the
/// strict-mode future-reserved words (`implements`, `interface`, `package`,
/// `private`, `protected`, `public`) and `async`/`await`/`let`/`static`/`yield`
/// are illegal as bare `IdentifierReference`s too and must be guarded.
pub(super) fn is_reserved_word(name: &str) -> bool {
    matches!(
        name,
        "break"
            | "case"
            | "catch"
            | "class"
            | "const"
            | "continue"
            | "debugger"
            | "default"
            | "delete"
            | "do"
            | "else"
            | "enum"
            | "export"
            | "extends"
            | "false"
            | "finally"
            | "for"
            | "function"
            | "if"
            | "import"
            | "in"
            | "instanceof"
            | "new"
            | "null"
            | "return"
            | "super"
            | "switch"
            | "this"
            | "throw"
            | "true"
            | "try"
            | "typeof"
            | "var"
            | "void"
            | "while"
            | "with"
            // Strict-mode reserved / contextual keywords (MJS is always strict).
            | "async"
            | "await"
            | "implements"
            | "interface"
            | "let"
            | "package"
            | "private"
            | "protected"
            | "public"
            | "static"
            | "yield"
    )
}

/// Whether `name` is a binding on every `global` object, so rewriting
/// `alias.NAME` → bare `NAME` cannot turn a property read that yields
/// `undefined` when the property is missing into a `ReferenceError`.
///
/// The output uses ES2015 syntax (`const`, arrows, templates), so it only runs
/// where the ES2015 globals exist; later ones (`BigInt`, `WeakRef`,
/// `SharedArrayBuffer`, …) are feature-detected through the global object and
/// stay qualified. Host names count only for the hosts `global` can be: a
/// worker has no `document`, Node has no `navigator`, and some windows lack
/// `caches`, `Notification` or `fetch`.
pub(super) fn is_always_defined_global(name: &str, global: GlobalObject) -> bool {
    let ecmascript = matches!(
        name,
        "Object" | "Function" | "Array" | "String" | "Boolean" | "Number" | "Symbol" | "Math" | "Date"
            | "RegExp" | "JSON" | "Error" | "EvalError" | "RangeError" | "ReferenceError" | "SyntaxError"
            | "TypeError" | "URIError" | "Promise" | "Proxy" | "Reflect" | "Map" | "Set" | "WeakMap"
            | "WeakSet" | "ArrayBuffer" | "DataView" | "Int8Array" | "Uint8Array" | "Uint8ClampedArray"
            | "Int16Array" | "Uint16Array" | "Int32Array" | "Uint32Array" | "Float32Array" | "Float64Array"
            | "parseInt" | "parseFloat" | "isNaN" | "isFinite" | "encodeURI" | "encodeURIComponent"
            | "decodeURI" | "decodeURIComponent" | "NaN" | "Infinity" | "undefined"
            // Timers and console exist in every browser context and in Node.
            | "console" | "setTimeout" | "setInterval" | "clearTimeout" | "clearInterval"
    );
    // Exposed to windows and to every kind of worker.
    let window_or_worker = || {
        matches!(
            name,
            "self"
                | "location"
                | "navigator"
                | "addEventListener"
                | "removeEventListener"
                | "dispatchEvent"
                | "atob"
                | "btoa"
                | "performance"
                | "crypto"
        )
    };
    let window = || {
        window_or_worker()
            || matches!(
                name,
                "window"
                    | "document"
                    | "history"
                    | "screen"
                    | "localStorage"
                    | "sessionStorage"
                    | "indexedDB"
                    | "getComputedStyle"
                    | "matchMedia"
                    | "requestAnimationFrame"
                    | "cancelAnimationFrame"
                    | "postMessage"
                    | "XMLHttpRequest"
                    | "WebSocket"
                    | "Blob"
                    | "File"
                    | "FileReader"
                    | "FormData"
                    | "URL"
                    | "Event"
                    | "CustomEvent"
                    | "Worker"
                    | "DOMParser"
            )
    };
    let node = || matches!(name, "process" | "Buffer" | "setImmediate" | "clearImmediate");
    ecmascript
        || match global {
            GlobalObject::Window => window(),
            GlobalObject::WindowOrWorker => window_or_worker(),
            GlobalObject::Node => node(),
            GlobalObject::Any => false,
        }
}

/// Whether the member expression currently being visited is the operand of a
/// `delete` (looking through any `(…)` wrappers oxc preserves). `delete bareName`
/// is a `SyntaxError` in strict mode, so such a member must stay qualified.
pub(super) fn is_delete_operand(ctx: &Ctx<'_>) -> bool {
    ctx.ancestors()
        .find_map(|a| match a {
            // Look through parentheses: `delete (Q.foo)` wraps the member.
            Ancestor::ParenthesizedExpressionExpression(_) => None,
            Ancestor::UnaryExpressionArgument(u) if u.operator().is_delete() => Some(true),
            // First non-paren, non-delete ancestor → not a delete operand.
            _ => Some(false),
        })
        .unwrap_or(false)
}
