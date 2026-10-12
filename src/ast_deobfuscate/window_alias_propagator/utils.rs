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
///
/// With `es5_only`, browser names are limited to those Internet Explorer 9,
/// the first ES5 browser, defines: no ES2015 globals (`Symbol`, `Promise`,
/// `Map`, typed arrays, …), no `console` (IE9 defines it only while its
/// developer tools are open), and no later host APIs (`crypto`, `atob`,
/// `Blob`, `requestAnimationFrame`, …), for output that is transpiled back to
/// ES5 or for bundles that feature-detect those names through the alias.
pub(super) fn is_always_defined_global(name: &str, global: GlobalObject, es5_only: bool) -> bool {
    let es5 = matches!(
        name,
        "Object"
            | "Function"
            | "Array"
            | "String"
            | "Boolean"
            | "Number"
            | "Math"
            | "Date"
            | "RegExp"
            | "JSON"
            | "Error"
            | "EvalError"
            | "RangeError"
            | "ReferenceError"
            | "SyntaxError"
            | "TypeError"
            | "URIError"
            | "parseInt"
            | "parseFloat"
            | "isNaN"
            | "isFinite"
            | "encodeURI"
            | "encodeURIComponent"
            | "decodeURI"
            | "decodeURIComponent"
            | "NaN"
            | "Infinity"
            | "undefined"
            | "setTimeout"
            | "setInterval"
            | "clearTimeout"
            | "clearInterval"
    );
    let es2015 = || {
        matches!(
            name,
            "Symbol" | "Promise" | "Proxy" | "Reflect" | "Map" | "Set" | "WeakMap" | "WeakSet" | "ArrayBuffer"
                | "DataView" | "Int8Array" | "Uint8Array" | "Uint8ClampedArray" | "Int16Array" | "Uint16Array"
                | "Int32Array" | "Uint32Array" | "Float32Array" | "Float64Array"
                // Console exists in every browser context and in Node.
                | "console"
        )
    };
    // Exposed to windows and to every kind of worker, by IE9 too.
    let window_or_worker_es5 = || {
        matches!(
            name,
            "self" | "location" | "navigator" | "addEventListener" | "removeEventListener" | "dispatchEvent"
        )
    };
    let window_or_worker = || window_or_worker_es5() || matches!(name, "atob" | "btoa" | "performance" | "crypto");
    let window_es5 = || {
        window_or_worker_es5()
            || matches!(
                name,
                "window"
                    | "document"
                    | "history"
                    | "screen"
                    | "localStorage"
                    | "sessionStorage"
                    | "getComputedStyle"
                    | "postMessage"
                    | "XMLHttpRequest"
                    | "Event"
                    | "CustomEvent"
                    | "DOMParser"
                    | "performance"
            )
    };
    let window = || {
        window_es5()
            || window_or_worker()
            || matches!(
                name,
                "indexedDB"
                    | "matchMedia"
                    | "requestAnimationFrame"
                    | "cancelAnimationFrame"
                    | "WebSocket"
                    | "Blob"
                    | "File"
                    | "FileReader"
                    | "FormData"
                    | "URL"
                    | "Worker"
            )
    };
    let node = || matches!(name, "process" | "Buffer" | "setImmediate" | "clearImmediate");
    match global {
        GlobalObject::Node => es5 || es2015() || node(),
        _ if es5_only => {
            es5 || match global {
                GlobalObject::Window => window_es5(),
                GlobalObject::WindowOrWorker => window_or_worker_es5(),
                GlobalObject::Node | GlobalObject::Any => false,
            }
        }
        GlobalObject::Window => es5 || es2015() || window(),
        GlobalObject::WindowOrWorker => es5 || es2015() || window_or_worker(),
        GlobalObject::Any => es5 || es2015(),
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
