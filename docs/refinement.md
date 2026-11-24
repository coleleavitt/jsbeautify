# The refinement loop

jsbeautify is developed mostly by coding agents, and most of the work is one
loop: **find a bug, pin it in a fixture, fix the pass, prove the fix, land it.**
This page says what each step uses so an agent can run the loop without a human
filling in the gaps. The rules every change must follow are in
[`AGENTS.md`](../AGENTS.md).

```text
   real bundles ──► find ──► reproduce ──► fix ──► verify ──► land
        ▲                                                     │
        └───────────── new bundles, new obfuscators ◄─────────┘
```

## 1. Find

Bugs come from three places.

- **Real-world input.** Run `jsbeautify -d` on a production bundle and keep
  the stderr log. A crash, a hang, a missing re-parse line, or output that does
  not behave like the input is a bug.
- **Hunts.** The `vuln-scan` skill drives the `codegraph-vuln-analyst`
  subagent over a subsystem and reports each bug with the call path that
  reaches it. `scripts/codeql.sh` runs CodeQL's security-and-quality suite
  locally.
- **Reviews.** The `hostile-input-reviewer` subagent asks what input makes
  the parser, a decoder, `vm_analysis` or a file writer panic, hang, exhaust
  memory or write outside the output directory.

## 2. Reproduce

Shrink the input to the smallest file that still shows the bug and put it under
`tests/fixtures/<bundler>/`. Add the test to the matching module in
`tests/integration/` (one aggregated target; don't add `tests/*.rs` files).
The test should fail before the fix.

## 3. Fix

- One pass per behavior. The `new-pass` skill scaffolds a pass and wires it
  into the engine.
- Passes resolve identity through Oxc's semantic model (`SymbolId`), not by
  name. Shadowing, `eval`, `with`, `this`, getters and evaluation order are
  where unsound rewrites hide.
- New parse or traversal entry points run through `stack::run_on_sized_stack`.

## 4. Verify

- **Soundness review.** After any change to a pass or renamer, run the
  `deobf-soundness-reviewer` subagent. Its question is whether the rewrite
  preserves behavior on *every* input, not just the fixture.
- **The gate.** `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test --all-targets` (the `gate` skill runs all three).
- **The output gate.** On real input:

  ```bash
  jsbeautify -d in.js -o out.js 2>deob.log
  grep -cE 'Phase (19.6|21.5): Generated output re-parse succeeded' deob.log  # 2
  node --check out.js
  diff <(node in.js 2>&1) <(node out.js 2>&1)   # when the input runs standalone
  ```

## 5. Land

One logical change per commit, with its fixture and test. A pass that is unsound
on some input is worse than no pass, so a fix that cannot be shown sound is not
landed.

## Agent tooling

| Path | What it is |
|---|---|
| [`AGENTS.md`](../AGENTS.md) | Build, gate, layout and correctness rules for every agent |
| `CLAUDE.md` | Claude Code additions (hooks, subagents, skills) |
| `.claude/agents/` | `deobf-soundness-reviewer`, `hostile-input-reviewer`, `codegraph-vuln-analyst` |
| `.claude/skills/` | `new-pass`, `gate`, `vuln-scan` |
| `.claude/hooks/` | rustfmt on edit, a stop-time clippy check, edits to `target/` refused |
| `scripts/codeql.sh` | Local CodeQL scan; results in `.codeql/` |
