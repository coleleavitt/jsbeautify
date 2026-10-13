# jsbeautify — Claude working notes

A JavaScript beautifier and AST deobfuscator built on Oxc. The CLI binary is
`jsbeautify`; the crate also exposes library APIs for beautification,
deobfuscation, bundle splitting/alignment, and static VM analysis.

## Build / test / lint

- The project has no `rust-toolchain.toml` and no package `rust-version` pin.
  Use a current Rust toolchain compatible with the pinned Oxc revision.
- `oxc_*` crates are pinned to one Git revision in `Cargo.toml`. Do not describe
  them as floating on `main`; update the revision and migration code together.
- Gate: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, then
  `cargo test --all-targets`.
- Integration tests use one declared aggregator target at
  `tests/integration/main.rs`; target it with `cargo test --test integration`.

## Architecture

- `src/ast_deobfuscate/engine/` owns the ordered pipeline. Its structural,
  cosmetic-A, and cosmetic-B arenas are separated by Phase 12.6 and 15.6
  codegen/re-parse checkpoints.
- `src/ast_deobfuscate/` owns the individual Oxc passes. Bundler-specific passes
  live in per-bundler folders. `bun/` holds the runtime helpers esbuild and
  Bun share (detection, the bundler they identify, renaming, annotation), CJS
  flattening, module handles, export/local renamers, and the alphabet.
- `src/vm_analysis/` performs bounded static JavaScript/Wasm inspection. It must
  never execute JavaScript or instantiate WebAssembly.
- `src/bin/jsbeautify_app/` owns CLI argument parsing and command runners.
- `src/cross_version/`, `src/sourcemap.rs`, `src/chunk_detector.rs`, and
  `src/chunk_splitter.rs` own alignment and bundle/source reconstruction.

## Correctness bar

Deobfuscation must preserve runtime behavior. Review transforms for resolved
symbol identity, shadowing, evaluation order, coercion, dynamic access/`eval`,
and `this` behavior. Every codegen/string-annotation boundary must continue to
reject invalid JavaScript rather than return it as success.

## Code navigation

Prefer CodeGraph for symbol definitions, call relationships, and impact. Use
literal search only for strings, comments, and log text. See `AGENTS.md`.
