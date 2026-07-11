# js-beautify-rs — Claude working notes

A fast JavaScript **beautifier + deobfuscator** built on `oxc`. The real target is
the **Bun-bundled Claude Code `cli.js`** — Bun-first priorities; generic obfuscator
passes are largely inert on it (see `GAP_ANALYSIS.md`). CLI binary: `jsbeautify`.

## Build / test / lint
- Toolchain: **nightly** is the active default (rustfmt `style_edition = "2024"` needs it).
- `oxc_*` crates are pulled from **git `main`** — APIs churn; use `context7` for current oxc docs.
- Gate before committing: **`/gate`** (= `cargo fmt --check` → `cargo clippy --all-targets -D warnings` → `cargo test`). Clippy runs `pedantic` + `nursery` — fix, don't `#[allow]`, unless narrowly justified.
- Integration tests: one aggregator target (`tests/integration/main.rs`), `cargo test --test integration`.

## Architecture (where things live)
- `src/ast_deobfuscate/` — the deobfuscation passes. Each is an `oxc_traverse::Traverse<DeobfuscateState>` impl following `pass.rs` (the `DeobfuscatePass` trait + `Phase` enum: Detection→Structural→Simplification→Cosmetic). Passes are registered in `mod.rs` and run by phase. Scaffold new ones with **`/new-pass`**.
- `src/analyzers/`, `src/wordlist/` — recon/extraction (note: ~13 `framework_recon` extractors are dead/mistargeted — see `FRAMEWORK_EXTRACTION_GAPS.md`).
- `src/db/` — bundled `rusqlite` project store (`~/.jsbeautify/projects/<name>/project.db`). Inspect via the `jsbeautify-db` MCP server.
- `src/proxy/` (MITM + CA), `src/server/` (daemon) — the untrusted-network boundary.

## Correctness bar (non-negotiable)
Deobfuscation must be **behavior-preserving**. Any new/changed pass or renamer is
reviewed for runtime-semantic soundness — scope-by-`SymbolId` (not name), reachable
`eval`/dynamic access poisoning, shadowing, `this`-aliasing, coercion/eval order.
Route such changes through the **`deobf-soundness-reviewer`** subagent; route
parser/proxy/decoder changes (adversarial input) through **`hostile-input-reviewer`**.

## Tooling in this repo (`.claude/`, gitignored via global ignore — local only)
- Hooks: rustfmt-on-edit, clippy gate on Stop, `ca.rs`/build-artifact edit guard.
- Subagents: `deobf-soundness-reviewer`, `hostile-input-reviewer`, `codegraph-vuln-analyst`.
- Skills: `/gate`, `/new-pass`, `/vuln-scan` (graph-driven bug hunt), `/codegraph-gaps`.
- MCP: `jsbeautify-db` (SQLite), `context7` (oxc/clap/tokio docs), plus the `codegraph_*` graph tools.

## Code navigation
Prefer **codegraph** (`codegraph_*` MCP tools) over grep for structural questions —
see `AGENTS.md`. `codegraph_arch <path>` maps a subsystem; `codegraph_xref <sym>`
finds all references; `codegraph_paths <from> <to>` traces reachability.

## Gotchas
- Don't commit crash dumps (`core.*`) or scratch binaries — gitignored, but they pile up.
- `src/bin/jsbeautify.rs::run()` is a known complexity hotspot (cyclomatic ~214) — refactor target, touch carefully.
