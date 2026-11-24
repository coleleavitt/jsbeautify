---
name: codegraph-gaps
description: >-
  Mine the agent's real tool-call history (.jfc/audit/runtime.jsonl + Claude Code
  transcripts) to find where it fell back to shell grep/cat/find for STRUCTURAL
  code questions that codegraph should have answered. Produces a ranked "what is
  codegraph missing" report that prioritizes which codegraph MCP tools to build.
  Use to measure codegraph coverage gaps, decide what to add to codegraph, or
  answer "what kind of bash are we actually running and why".
disable-model-invocation: true
---

# codegraph-gaps — empirical codegraph coverage gap-finder

This is the **meta-loop**: instead of guessing what codegraph should do, it reads
what the agent *actually did* and finds the structural questions it answered with
the shell because codegraph couldn't (or the model didn't reach for it).

## Run

```bash
bash .claude/skills/codegraph-gaps/gaps.sh [audit.jsonl] [transcript-dir]
```
Defaults: `.jfc/audit/runtime.jsonl` and `~/.claude/projects/<sanitized-cwd>/`.

## What it reports
1. **Native tool mix** — `codegraph_*` vs `Read/Grep/Glob` across transcripts. Every
   structural `Read/Grep` is a codegraph miss.
2. **Bash by purpose** — buckets each Bash call and maps it to the codegraph tool
   that *should* serve it (grep→search/xref, cat/sed→node/explore, find/ls→files/arch).
3. **Structural %** — the headline: what fraction of shell calls did codegraph's job.
4. **Top grep terms** — recurring symbol searches = `codegraph_search`/`codegraph_xref` demand.
5. **Most re-read files** — = `codegraph_node`/`codegraph_explore` demand.

## How to act on it
- A bucket with a matching codegraph tool but high shell count = **ergonomics gap**
  (the tool exists; fix the prompt / AGENTS.md so the model reaches for it).
- A bucket with **no** good codegraph equivalent = **coverage gap** (build the tool).
  The recurring one is *architecture mapping* (grep+read bursts) → `codegraph_arch`.

Feed the output into the codegraph-rs task list (codegraph_arch / codegraph_paths /
codegraph_xref / codegraph_vuln). Re-run after shipping a tool to confirm the
corresponding shell bucket shrinks.
