# Next steps

This file lists only work that remains after the current Oxc migration and
static VM-analysis implementation. Completed JSON, optional-chaining, nullish,
`switch(true)`, dead-variable, and VM-analysis tasks were removed; their source
and tests are the record of completion.

## 1. Stabilize the framework-recon interface

`src/analyzers/framework_recon/` now aggregates existing text-in/result-out
extractors, and `jsbeautify framework-recon` emits its versioned JSON report.
It is intentionally separate from `AstDeobfuscator`'s mutation pipeline.

Before adding more extractors:

- add stdin coverage to the existing output-file and oversized-input CLI tests;
- add broader multi-framework and false-positive fixtures;
- document `FrameworkReconReport` schema compatibility;
- decide which application-specific detectors should require explicit opt-in.

The current inventory and evidence are in `FRAMEWORK_EXTRACTION_GAPS.md`.

## 2. Harden existing framework extractors before broadening coverage

Prioritize verified defects in compiled modules:

- Generalize `mfe_registry_extractor` beyond its single-spa/application-specific
  assumptions, or rename it to match its actual scope.
- Replace application-specific LaunchDarkly and HTTP-interceptor assumptions
  with generic, tested signatures.
- Bring the RxJS operator catalog and Angular instruction coverage in line with
  the framework versions the project intends to support.
- Make webpack runtime/module recognition structural rather than dependent on
  one bundle's names.

Keep the CLI report schema versioned and require integration tests for every
new detector or output-shape change.

## 3. Extend static VM analysis without execution

`analyze-vm` currently performs bounded static JavaScript/Wasm inspection,
validates embedded Wasm, and can extract literal modules without overwriting
existing files. Useful remaining work must preserve those invariants:

- add ISA-specific decoding only when an instruction format can be proven from
  static evidence;
- improve provenance from JavaScript decoder/dispatcher sites to report items;
- add fixtures for each supported VM family and malformed/limit case;
- keep JavaScript execution and WebAssembly instantiation out of the analyzer.

## 4. Keep the verification gate current

Use the repository's upstream-tracking Rust workflow: no project toolchain file
or `rust-version` pin, and a pinned Oxc Git revision in `Cargo.toml`. For every
change run:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

When updating Oxc, update the revision and migration code in the same change and
rerun the complete gate.
