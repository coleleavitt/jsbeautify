---
name: new-pass
description: >-
  Scaffold a new AST deobfuscation pass in jsbeautify. Creates the pass module
  under src/ast_deobfuscate/ as an oxc Traverse impl over DeobfuscateState, wires
  it into ast_deobfuscate/mod.rs and the engine (field, constructor, and a
  traverse call in the right engine phase), and adds the standard
  run_<name>(code) -> String test harness plus adversarial soundness tests. Use
  when adding a new deobfuscation / AST transform pass, normalizer, or extractor.
---

# new-pass — scaffold a deobfuscation pass

Use this to add a behavior-preserving AST transform following the existing conventions.
Read one nearby pass (e.g. `boolean_literals.rs` or `array_unpack.rs`) and the engine
file that will run it before writing. Match their style, not a generic template.

## Inputs to settle first
- **Name** in `snake_case` (file: `src/ast_deobfuscate/<name>.rs`) and the struct in
  `PascalCase` (e.g. `array_unpack.rs` → `ArrayUnpacker`).
- **Where it runs.** The pipeline lives in `src/ast_deobfuscate/engine/`, split by
  stage: `structural_early.rs`, `structural_bun.rs`, `structural_mid.rs`,
  `structural_late.rs`, `structural_late_local.rs` (all before the Phase 12.6
  codegen/re-parse checkpoint), then `cosmetic.rs` (cosmetic-A and cosmetic-B,
  split by the Phase 15.6 checkpoint), then `final_output.rs`. Phase numbers are
  log labels, not an enum. Find the group of passes doing similar work with
  `grep -n '\[DEOBFUSCATE\] Phase' src/ast_deobfuscate/engine/*.rs` and join it.
- **What AST shape it matches and what it rewrites it to** — and the soundness guard that
  makes the rewrite safe (scope by `SymbolId`, bail on reachable `eval`/dynamic access,
  preserve `this`, etc).

## Steps

### 1. Create `src/ast_deobfuscate/<name>.rs`
A pass is a struct with a rewrite counter and an oxc `Traverse` impl over
`DeobfuscateState`. For a single counter, `counter_pass!` (from `pass.rs`) generates
`new()`, the getter, and `Default`:

```rust
use oxc_ast::ast::Expression;
use oxc_traverse::{Traverse, TraverseCtx};

use crate::ast_deobfuscate::pass::counter_pass;
use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct <Pascal> {
    rewrites: usize,
}

counter_pass!(<Pascal>, rewrites, rewrites);

impl<'a> Traverse<'a, DeobfuscateState> for <Pascal> {
    // Prefer exit_* so children are already transformed. Match the narrow shape,
    // bail early on anything unproven, and count every rewrite.
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        // ... guard, then rewrite ...
        deob_trace!("[AST] <name>: ...");
        self.rewrites += 1;
    }
}
```

With more than one counter, write `new()`, the getters, and `Default` by hand, as
`boolean_literals.rs` does.

### 2. Wire it into `src/ast_deobfuscate/mod.rs`
- Add `pub(crate) mod <name>;` to the alphabetical module list.
- Add `pub(crate) use <name>::<Pascal>;` to the re-export block.

### 3. Wire it into the engine
- `engine/mod.rs`: add `<Pascal>` to the `use super::{...}` list and a field
  `<snake_field>: <Pascal>,` to `AstDeobfuscator`.
- `engine/lifecycle.rs`: add it to the `use super::{...}` list and initialize the
  field in `AstDeobfuscator::new()` (`<snake_field>: <Pascal>::new(),`).
- The chosen engine stage file: add the call next to its neighbours, in the same
  logging style:

  ```rust
  eprintln!("[DEOBFUSCATE] Phase N: Running <name>");
  traverse_mut_with_ctx(&mut self.<snake_field>, program, &mut ctx);
  ```

- If that phase has a `phase_N_rewrites()` sum (see `structural_late.rs`), add the
  new counter to it. The engine compares that sum before and after the phase to
  decide whether it may reuse the old scoping; a rewrite it does not count leaves
  later passes working from stale scoping.

### 4. Add tests (same file, `#[cfg(test)] mod tests`)
Reuse the project's harness shape:

```rust
use super::*;
use crate::ast_deobfuscate::{DeobfuscateState, semantic};
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_span::SourceType;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

fn run_<name>(code: &str) -> String {
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
    let mut program = ret.program;
    let mut pass = <Pascal>::new();
    let scoping = semantic::scoping(&program);
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
    traverse_mut_with_ctx(&mut pass, &mut program, &mut ctx);
    Codegen::new().build(&program).code
}
```

Cover: the happy path, AND the **soundness counterexamples** — shadowing, reachable
`eval`/dynamic property access, `this`/receiver, getter/setter side effects, coercion/eval
order. A pass without adversarial tests is not done.

### 5. Optional fixture
For a realistic end-to-end case, drop a sample under `tests/fixtures/obfuscated/` and add
a check to `tests/integration/deobfuscate.rs` that loads it with
`fixtures::obfuscated("<file>.js")`.

### 6. Verify
Run `/gate` (or `cargo test` + `cargo clippy --all-targets -- -D warnings`). For any pass
that renames, inlines, removes, or folds, route the change through the
`deobf-soundness-reviewer` subagent before considering it complete.
