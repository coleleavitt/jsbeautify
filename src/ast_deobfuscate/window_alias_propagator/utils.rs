use oxc_ast::ast::Expression;
use oxc_semantic::Scoping;
use oxc_traverse::Ancestor;

use super::Ctx;

pub(super) fn is_global_like(expr: &Expression<'_>, scoping: &Scoping) -> bool {
    // NOTE: `this` is deliberately NOT global-like. `var self = this; self.foo`
    // inside a method/class reads an instance property — rewriting it to bare
    // `foo` silently turns it into a global lookup (a real miscompile). Only the
    // genuine global objects qualify, read through an unbound name: a local or
    // parameter called `window` is an ordinary value.
    matches!(expr, Expression::Identifier(id)
        if matches!(id.name.as_str(), "window" | "globalThis" | "global" | "self")
            && id.reference_id.get().is_some_and(|r| scoping.get_reference(r).symbol_id().is_none()))
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

/// Global-object properties that are reliably defined as global *bindings*, so
/// rewriting `globalAlias.NAME` → bare `NAME` cannot turn a safe property read
/// (a missing key yields `undefined`) into a `ReferenceError` (an unresolved
/// bare identifier throws).
///
/// Two tiers: `ECMAScript` spec globals (present in every JS environment) and the
/// common browser host globals. The pass only fires when the alias is a
/// browser-like global object (`window`/`self`/`globalThis`/`global`), so the
/// browser tier matches the code being deobfuscated. Any property name NOT on
/// this list is left qualified — `alias.unknownProp` stays `alias.unknownProp`
/// rather than risk a runtime `ReferenceError`.
pub(super) fn is_always_defined_global(name: &str) -> bool {
    matches!(
        name,
        // ── ECMAScript spec globals (universal) ──
        "globalThis" | "Object" | "Array" | "Function" | "Boolean" | "Symbol"
        | "Error" | "EvalError" | "RangeError" | "ReferenceError" | "SyntaxError"
        | "TypeError" | "URIError" | "AggregateError" | "Number" | "BigInt" | "Math"
        | "Date" | "String" | "RegExp" | "JSON" | "Promise" | "Proxy" | "Reflect"
        | "Map" | "Set" | "WeakMap" | "WeakSet" | "WeakRef" | "ArrayBuffer"
        | "SharedArrayBuffer" | "DataView" | "Atomics"
        | "Int8Array" | "Uint8Array" | "Uint8ClampedArray" | "Int16Array"
        | "Uint16Array" | "Int32Array" | "Uint32Array" | "Float32Array"
        | "Float64Array" | "BigInt64Array" | "BigUint64Array"
        | "parseInt" | "parseFloat" | "isNaN" | "isFinite"
        | "encodeURI" | "encodeURIComponent" | "decodeURI" | "decodeURIComponent"
        | "NaN" | "Infinity" | "undefined"
        // ── Browser host globals (present when window/self exists) ──
        | "window" | "self" | "document" | "location" | "navigator" | "history"
        | "screen" | "console" | "localStorage" | "sessionStorage" | "indexedDB"
        | "fetch" | "XMLHttpRequest" | "WebSocket" | "EventSource"
        | "setTimeout" | "setInterval" | "clearTimeout" | "clearInterval"
        | "requestAnimationFrame" | "cancelAnimationFrame" | "queueMicrotask"
        | "structuredClone" | "atob" | "btoa" | "crypto" | "performance"
        | "addEventListener" | "removeEventListener" | "dispatchEvent"
        | "postMessage" | "getComputedStyle" | "matchMedia"
        | "URL" | "URLSearchParams" | "Blob" | "File" | "FileReader" | "FormData"
        | "Headers" | "Request" | "Response" | "AbortController" | "AbortSignal"
        | "TextEncoder" | "TextDecoder" | "Event" | "CustomEvent" | "Worker"
        | "Notification" | "caches" | "customElements" | "DOMParser"
    )
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
