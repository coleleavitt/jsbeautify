---
name: codegraph-evolve
description: >-
  The self-improvement loop for the codegraph toolchain. Mines the agent's own
  tool-call history (via /codegraph-gaps) to find capabilities the agent had to
  fake with grep/read, then AUTHORS the missing capability — a new skill/subagent
  (safe tier) or a new codegraph_* MCP tool (gated tier) — and re-runs the
  gap-finder to confirm the gap closed. Use to extend codegraph based on how it is
  actually used, or when asked to "make codegraph self-improve / add tools based on
  what it's missing". Measures success by a real metric (structural-shell-call %),
  not vibes.
disable-model-invocation: true
---

# codegraph-evolve — the self-improvement loop

Closes the loop the rest of the program set up: **find what's missing → build it →
prove it shrank the gap.** This is what makes the toolchain self-learning instead of
a one-shot.

```
/codegraph-gaps  →  pick the top gap  →  build it (Tier 1 or Tier 2)  →  verify  →  /codegraph-gaps (confirm the bucket shrank)
```

## Step 1 — Find the gap
Run `/codegraph-gaps`. It buckets the agent's real Bash/Read/Grep history and maps
each structural fallback to the codegraph tool that should serve it. Pick the
highest-count bucket that has **no** good codegraph equivalent (a *coverage* gap),
or one with high shell-use despite an existing tool (an *ergonomics* gap → fix the
prompt/AGENTS.md, not new code).

## Step 2a — Tier 1: author a skill or subagent (SAFE — do this first)
For an ergonomics gap or a recurring analysis pattern, generate a `.claude/skill` or
`.claude/agents/*.md` tailored to it. Pure prompt/markdown — nothing compiles, nothing
can corrupt a shared binary, so this tier is always safe to ship. Model it on the
existing `codegraph-vuln-analyst` / `vuln-scan` artifacts. (This is T13.)

## Step 2b — Tier 2: author a new codegraph MCP tool (AMBITIOUS — hard-gated)
For a genuine coverage gap, add a `codegraph_*` MCP tool to **codegraph-rs**,
following the proven recipe in `codegraph-rs/CLAUDE.md`: dispatch arm in
`ToolHandler::execute()` + a `handle_x` handler + a `ToolDefinition` in `tools()` +
bump `tool_definition_json_is_wire_compatible_with_ts` + a functional test in
`tests/mcp_tools_test.rs`. (This is T14.)

**Non-negotiable gates — never auto-merge generated Rust into the shared tool:**
1. `cargo build` (codegraph-rs) — must compile.
2. `cargo test` incl. the new functional test — must pass.
3. Route the diff through a review subagent (`code-reviewer` / soundness lens).
4. Only then `cargo install --path .` + restart the codegraph daemon (kill the stale
   per-project `codegraph serve --mcp --path …` process; a fresh one respawns from the
   new binary; reconnect the MCP client to re-fetch the tool list).

## Step 3 — Verify the loop closed (T15)
Re-run `/codegraph-gaps`. The bucket you targeted should shrink (fewer structural
shell calls, more `codegraph_*` use). That delta — a real number — is the proof the
system learned. Log what was built and the before/after metric. If the bucket didn't
move, the new tool isn't being reached for → it's an ergonomics problem (fix the tool
description / AGENTS.md), loop again.

## Guardrails
- Tier 1 before Tier 2 — most gaps are ergonomics, not capability.
- One gap per iteration; verify before the next. Don't batch unverified generated code.
- A generated tool that fails any gate is discarded, not patched into place.
- Stop when the top gaps are all ergonomics (no new capability warranted).
