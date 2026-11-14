# jsbeautify — agent guide

A JavaScript beautifier and AST deobfuscator built on Oxc. The CLI binary is
`jsbeautify` (`src/bin/jsbeautify.rs`); the crate also exposes library APIs for
beautification, deobfuscation, bundle splitting/alignment, and static VM
analysis. `jsbeautify --help` lists the subcommands and flags.

This file is the shared guide for every coding agent. `CLAUDE.md` imports it
and adds only Claude Code specifics.

## Build / test / lint

- The project has no `rust-toolchain.toml` and no package `rust-version` pin.
  Use a current Rust toolchain compatible with the pinned Oxc revision.
  `rustfmt.toml` sets `style_edition = "2024"`.
- `oxc_*` crates come from crates.io, all on one release (see `Cargo.toml`).
  Bump every `oxc_*` crate together, with the migration code, in one commit.
  Git or path dependencies cannot be published, so do not commit them.
- Gate, in order: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  then `cargo test --all-targets`. Clippy runs `pedantic` and `nursery`; fix
  lints rather than adding `#[allow]`, and scope any allow narrowly.
- Integration tests use one declared aggregator target at
  `tests/integration/main.rs`; target it with `cargo test --test integration`.
  Add new integration files as modules there, not as new `tests/*.rs` targets.
  Input fixtures live under `tests/fixtures/<bundler>/`.
- Heap profiling: `cargo build --profile profiling --features dhat-heap`.

## Repository layout

- `src/ast_deobfuscate/engine/` owns the ordered pipeline. Its structural,
  cosmetic-A, and cosmetic-B arenas are separated by Phase 12.6 and 15.6
  codegen/re-parse checkpoints.
- `src/ast_deobfuscate/` owns the individual Oxc passes. Bundler-specific passes live in per-bundler
  folders (`webpack/`, `turbopack/`, `angular/`, `akamai/`, `bun/`). `bun/`
  holds the runtime helpers esbuild and Bun share (detection, the bundler they
  identify, renaming, annotation), CJS flattening, module handles,
  export/local renamers, and the alphabet.
- `src/beautifier/` owns plain beautification and js-beautify-compatible
  output layout. `src/options.rs` holds the formatting and parse options.
- `src/project/` owns `--project` mode: deobfuscating every file of a
  code-split build together with cross-file names.
- `src/vm_analysis/` performs bounded static JavaScript/Wasm inspection. It must
  never execute JavaScript or instantiate WebAssembly.
- `src/inspect/` backs the `ast`, `scope`, and `cfg` subcommands.
- `src/bin/jsbeautify_app/` owns CLI argument parsing and command runners.
- `src/cross_version/`, `src/sourcemap.rs`, `src/chunk_detector.rs`,
  `src/chunk_splitter.rs`, `src/turbopack_extract.rs`,
  `src/webpack_module_extractor/`, and `src/bun_extract.rs` own alignment and
  bundle/source reconstruction.
- `src/llm.rs` is the OpenAI-compatible client for `--rename-llm`.
- `src/stack.rs` runs parsing and AST work on a thread whose stack is sized
  from the input (`run_on_sized_stack`), so deep nesting cannot overflow the
  stack. Run new parse or traversal entry points through it.
- `docs/` holds design notes, reviews, and research write-ups. They are
  historical; the code is the authority when they disagree.
- `research/` holds per-target analysis scripts and findings.
- `frontend/` is a separate Firefox DevTools extension (Node/JS) that dumps a
  page's scripts, source maps, and assets.

## Correctness bar

Deobfuscation must preserve runtime behavior. Review transforms for resolved
symbol identity, shadowing, evaluation order, coercion, dynamic access/`eval`,
and `this` behavior. Every codegen/string-annotation boundary must continue to
reject invalid JavaScript rather than return it as success.

Inputs are hostile: obfuscated or malicious JavaScript must not make the tool
panic, hang, or exhaust memory.

## Code navigation

Prefer CodeGraph for symbol definitions, call relationships, and impact. Use
literal search only for strings, comments, and log text.

<!-- CODEGRAPH_START -->
## CodeGraph

This project has a CodeGraph MCP server (`codegraph_*` tools) configured. CodeGraph is a tree-sitter-parsed knowledge graph of every symbol, edge, and file. Reads are sub-millisecond and return structural information grep cannot.

### When to prefer codegraph over native search

Use codegraph for **structural** questions -- what calls what, what would break, where is X defined, what is X's signature. Use native grep/read only for **literal text** queries (string contents, comments, log messages) or after you already have a specific file open.

| Question | Tool |
|---|---|
| "Where is X defined?" / "Find symbol named X" | `codegraph_search` |
| "What calls function Y?" | `codegraph_callers` |
| "What does Y call?" | `codegraph_callees` |
| "What would break if I changed Z?" | `codegraph_impact` |
| "Show me Y's signature / source / docstring" | `codegraph_node` |
| "Give me focused context for a task/area" | `codegraph_context` |
| "See several related symbols' source at once" | `codegraph_explore` |
| "What files exist under path/" | `codegraph_files` |
| "Is the index healthy?" | `codegraph_status` |

### Rules of thumb

- **Answer directly -- don't delegate exploration.** For "how does X work" / architecture / trace questions, answer with 2-3 codegraph calls: `codegraph_context` first, then ONE `codegraph_explore` for the source of the symbols it surfaces. Codegraph IS the pre-built index, so spawning a separate file-reading sub-task/agent -- or running a grep + read loop -- repeats work codegraph already did and costs more for the same answer.
- **Trust codegraph results.** They come from a full AST parse. Do NOT re-verify them with grep -- that's slower, less accurate, and wastes context.
- **Don't grep first** when looking up a symbol by name. `codegraph_search` is faster and returns kind + location + signature in one call.
- **Don't chain `codegraph_search` + `codegraph_node`** when you just want context -- `codegraph_context` is one call.
- **Don't loop `codegraph_node` over many symbols** -- one `codegraph_explore` call returns several symbols' source grouped in a single capped call, while each separate node/Read call re-reads the whole context and costs far more.
- **Index lag**: the file watcher debounces ~500ms behind writes; don't re-query immediately after editing a file in the same turn.

### If `.codegraph/` doesn't exist

The MCP server returns "not initialized." Ask the user: *"I notice this project doesn't have CodeGraph initialized. Want me to run `codegraph init -i` to build the index?"*
<!-- CODEGRAPH_END -->
