---
name: vuln-scan
description: >-
  Hunt vulnerabilities/bugs in your own Rust code using codegraph's graph tools +
  the codegraph-vuln-analyst subagent. Maps a target subsystem, traces source→sink
  reachability chains, runs the codegraph vuln engine as a lead generator, and
  reasons against dynamically-discovered bug classes (CWE/NVD/CVSS). Use to audit a
  subsystem or entrypoint, "find bugs in <area>", or do a security pass before a
  release. Requires the codegraph MCP server (with codegraph_arch/paths/xref/vuln).
disable-model-invocation: true
---

# vuln-scan — graph-driven vulnerability hunt

Orchestrates a real bug hunt over your own code. Give it a target (a subsystem path
like `src/sourcemap`, an entrypoint symbol, or "whole repo"); it drives the
`codegraph-vuln-analyst` subagent against the live codegraph tools.

## Preconditions
- codegraph MCP server running with the extended tools: `codegraph_arch`,
  `codegraph_paths`, `codegraph_xref`, `codegraph_vuln`. (Verify with a
  `codegraph_arch` call; if the tools are missing, the server needs a restart after
  `cargo install --path <codegraph-rs>`.)
- The project is codegraph-indexed (`.codegraph/` present).

## Steps
1. **Frame the target.** Resolve the subsystem path(s). For "whole repo", first run
   `codegraph_arch` at the root to pick the highest-risk areas (the
   untrusted-input boundary: parser, decoders, `src/vm_analysis`, the output
   writers that turn bundle names into paths, `src/llm.rs`).
2. **Lead generation.** Run `codegraph analyze vuln` (CLI) or `codegraph_vuln`
   (MCP) for the engine's structural anomalies — a starting list, not the answer.
3. **Dispatch the analyst.** For each target subsystem, spawn the
   `codegraph-vuln-analyst` subagent (run several in parallel for distinct areas).
   It maps the area, traces source→sink chains with `codegraph_paths`, and
   dynamically classifies each finding (CWE/NVD/CVSS via the `vuln-researcher`
   skill + web search).
4. **Verify + dedup.** Drop findings without a real reachability chain or a concrete
   trigger. Dedup across analysts by (file, sink, class).
5. **Report.** Produce a ranked report (severity, CWE, chain, trigger, CVSS, fix).

## Scope notes
- **Rust self-audit (default):** sources = parsed JS, source maps, LLM responses; sinks =
  panic/`unwrap`/index, `unsafe`, integer truncation/overflow, fs paths, `Command`.
- **Deobfuscated JS (separate pass):** a different source/sink set (DOM-XSS sinks,
  `eval`/`Function`, prototype pollution, exfil) — see the JS sink-catalog task.

Pair findings that touch deobfuscation or parser-facing code with the
`deobf-soundness-reviewer` / `hostile-input-reviewer` subagents for a second lens.
