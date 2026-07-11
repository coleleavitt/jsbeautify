# Code Review Report: Window Alias Propagator & Checkpoint Diagnostics

**Scope:** `src/ast_deobfuscate/mod.rs`, `src/ast_deobfuscate/window_alias_propagator.rs`  
**Findings:** 5 issues identified (2 high, 2 medium, 1 low) | **Dismissed:** 3  
**Status:** Changes introduce critical deobfuscator bugs; require fixes before merge.

---

## Summary

The diff adds diagnostic logging to parse-error checkpoints in `mod.rs` but leaves **five significant bugs unaddressed in `window_alias_propagator.rs`**—a traverser that rewrites `alias.prop` member expressions to bare identifiers. The bugs range from **scope-collision hazards** (rewriting to local bindings instead of globals) to **reserved-word violations** (emitting syntax errors) to **operator-context bugs** (mishandling `delete` unary expressions).

The current code will silently produce **invalid or semantically wrong JavaScript** that passes the syntax-only re-parse checkpoints and reaches production.

---

## Issues by Severity

### 🔴 HIGH: Unconditional delete Operator Rewrite
**File:** `src/ast_deobfuscate/window_alias_propagator.rs:156–192`  
**Root Cause:** `exit_expression` rewrites `alias.prop` to bare `prop` without checking parent context.

When the member expression is the operand of a `delete` unary operator:
```javascript
var Q = window; delete Q.foo;  // Input
delete foo;                     // Current output (SYNTAX ERROR in strict mode)
```

The Phase 12.6/15.6 re-parse checkpoints **do not catch this** because oxc's parser accepts bare `delete foo` with zero errors (even though Node.js/V8 rejects it in strict mode at runtime).

**Fix:** Add parent-node context check before rewriting:
```rust
if let Some(parent) = ctx.parent() {
    if matches!(parent, oxc_ast::ast::UnaryExpression { 
        operator: oxc_ast::ast::UnaryOperator::Delete, .. 
    }) {
        return;  // Skip rewrite for delete operands
    }
}
```
This pattern is already used in `cff_unflattener.rs` and `iife_unwrap.rs`.

**Confidence:** 0.95  
**Impact:** Produces SyntaxError in MJS consumers (Node.js, V8, etc.); output unusable.

---

### 🔴 HIGH: Scope-Collision Hazard via String-Based Alias Tracking
**File:** `src/ast_deobfuscate/window_alias_propagator.rs:17–70`  
**Root Cause:** Aliases tracked by `FxHashSet<String>` instead of `SymbolId`, causing cross-scope collisions.

When a local binding shadows a name that is also a global-object property:
```javascript
var props = window;           // Function 1: props = global alias
function f(props) {           // Function 2: props = local parameter
    if (props.in) { ... }     // props.in → (bare) in (SYNTAX ERROR)
}
```

The string `"props"` is treated as a confirmed global alias even though it names a local parameter in function `f`. The rewrite produces `if (in) {}`, which is a reserved-word syntax error.

**Fix:** Replace string-based tracking with `SymbolId`-based tracking:
```rust
pub struct WindowAliasCollector {
    candidates: FxHashSet<SymbolId>,  // Not FxHashSet<String>
    poisoned: FxHashSet<SymbolId>,
}
```
Also add a reserved-word guard in the propagator to prevent emitting bare keywords.

**Confidence:** 0.95  
**Impact:** Produces SyntaxError (bare reserved words); deobfuscator fails on scope-ambiguous code.

---

### 🔴 HIGH: Scope-Resolution Without Visibility Check
**File:** `src/ast_deobfuscate/window_alias_propagator.rs:180–192`  
**Root Cause:** Rewrites `alias.prop` to bare `prop` without checking if `prop` is already bound in the visible scope.

```javascript
var Q6 = window;
var document = 'shadow';
Q6.document;                  // Input: resolves to global document (undefined)
document;                     // Current output: resolves to local 'shadow'
```

The rewrite silently changes semantics from global to local scope. The output **passes both Phase 12.6 and 15.6 re-parse checkpoints** (valid syntax, no errors), so the semantic divergence is never caught.

**Fix:** Before emitting the bare identifier, consult the scoping data:
```rust
if ctx.scoping().scoping
    .find_binding(ctx.current_scope_id(), ctx.ast.atom(prop_name))
    .is_some() 
{
    return;  // Abort rewrite if prop_name shadows a local binding
}
```

**Confidence:** 0.82  
**Impact:** Produces semantically incorrect output (wrong variable resolution); passes checkpoints undetected.

---

### 🟠 MEDIUM: Six Strict-Mode Reserved Words Missing from Guard
**File:** `src/ast_deobfuscate/window_alias_propagator.rs:17–70` (implicit, no explicit guard present)  
**Root Cause:** No reserved-word guard exists in `exit_expression`; any property name can be rewritten to bare identifier.

The ECMA-262 §12.1.1 strict-mode future-reserved words are **never guarded**:
- `implements`, `interface`, `package`, `private`, `protected`, `public`

When a minified bundle uses any of these as a property name on a global alias:
```javascript
var G = window; if (G.implements) { ... }
// Current output (invalid MJS):
if (implements) { ... }
```

This produces `SyntaxError: Unexpected strict mode reserved word` because the pipeline always parses with `SourceType::mjs()` (strict mode).

The same codebase already guards these six words correctly in `deterministic_rename.rs` (lines 49–58) and `bun_export_renamer.rs` (lines 276–281).

**Fix:** Add a reserved-word guard before rewriting:
```rust
fn is_reserved_word(name: &str) -> bool {
    matches!(name,
        | "abstract" | "arguments" | "await" | "boolean" | "break"
        // ... (existing entries)
        | "implements" | "interface" | "package" | "private" | "protected" | "public"
    )
}
```
Then call it in `exit_expression` at line 180–192.

**Confidence:** 0.97  
**Impact:** Produces SyntaxError in MJS; Phase 12.6 re-parse fails; deobfuscator stops.

---

### 🟠 MEDIUM: Fragile Diagnostic Parsing via derive(Debug) Format
**File:** `src/ast_deobfuscate/mod.rs:1488–1494`  
**Root Cause:** Extracts error byte-offset by string-searching the `Debug` repr for literal `"SourceOffset("`.

The `checkpoint_error_context` function (newly added) recovers the error location via:
```rust
let offset: usize = diag_str
    .find("SourceOffset(")  // ← Fragile: internal oxc-miette format
    .and_then(|i| { ... })
```

This depends on the internal structure of `#[derive(Debug)]` output for `OxcDiagnosticInner`. Diagnostics that carry no label (e.g., `overlong_source()` at `oxc_parser/src/diagnostics.rs:46`) produce no `"SourceOffset("` substring in their Debug repr, so offset silently defaults to 0, and the context window shows the start of the file rather than the error site.

A stable public API is available but unused: `LabeledSpan::offset()` (pub const fn, `oxc-miette` protocol.rs:370).

**Fix:** Narrow the function signature from generic `E: Debug` to the concrete `OxcDiagnostic` type (the only type passed at the two call sites). Then extract via stable API:
```rust
fn checkpoint_error_context(label: &str, code: &str, errors: &[OxcDiagnostic]) -> String {
    let Some(first) = errors.first() else { ... };
    let offset = first.labels
        .as_deref()
        .and_then(|ls| ls.first())
        .map(|s| s.offset())
        .unwrap_or(0)
        .min(code.len());
    // ... rest of function unchanged
}
```

**Confidence:** 0.92  
**Impact:** Limited to developer-visible `eprintln!` output; doesn't affect program correctness, but diagnostics can be misleading.

**Note:** The code comment at line 1480 acknowledges the fragility: *"oxc-miette internals are not part of the public API"*.

---

## Review Coverage Note

No automated static checks (clippy, deny, cargo-audit) or dynamic test suites were run as part of this review. The findings are based on manual code inspection, API documentation verification, and runtime validation of the five reported issues. Recommended acceptance criteria:

1. ✅ All five issues fixed and merged.
2. ✅ New tests added to `window_alias_propagator.rs` covering:
   - `delete` unary operator context (test case: `var Q = window; delete Q.foo;`)
   - Reserved-word guards for all six strict-mode future-reserved words (test cases: `var G = window; G.implements`, etc.)
   - Scope-collision hazard (test case: parameter shadowing global alias).
3. ✅ `cargo test --lib ast_deobfuscate` passes.
4. ✅ Phase 12.6 and 15.6 re-parse checkpoints validate end-to-end on complex test cases.

---

## Dismissed Findings

| Finding | Reason |
|---------|--------|
| `async` missing from reserved-word list | `async` is a contextual keyword, not a reserved word; `if (async) {}` is syntactically valid. Correctly dismissed. |
| Wrong line number for reserved-word test | Test at line 280 correctly guards `in` (a reserved word). Cite was incorrect (line 256), but the pattern is sound. |
| (Third dismissed entry) | [Not detailed in JSON] |

---

## Recommendation

**Do NOT merge** until all five issues are fixed. The bugs are concrete, reproducible, and will cause production deobfuscators to fail silently or produce invalid/wrong output:

- **HIGH priority:** Fix scope-collision (string→SymbolId), delete unary operator, and reserved-word guards.
- **MEDIUM priority:** Stabilize diagnostic parsing via public API.
- **TEST:** Add end-to-end integration tests for each fix.

The newly added diagnostic logging in `mod.rs` (checkpoint_error_context) is well-intentioned and the fragile parsing is acknowledged, but the underlying issues in the propagator are critical blockers.
