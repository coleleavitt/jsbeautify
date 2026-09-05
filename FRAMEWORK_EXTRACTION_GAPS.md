# Framework-extraction status and gaps

This document describes the current source tree. It separates **compiled and
tested** from **reachable through a supported product interface**.

## Current architecture

Framework/build-tool modules are declared from
`src/ast_deobfuscate/mod.rs`, so they compile with the library and their unit
tests run. Most are text-oriented extract/format helpers, not Oxc mutation
passes. Examples include:

- Angular metadata, template, and route extraction;
- NgRx action/effect extraction and RxJS annotation;
- LaunchDarkly, interceptor, and registry inspection;
- webpack runtime decoding and module-package recognition;
- bundle-diff and Transmit SDK helpers.

They are **not** registered in `AstDeobfuscator`. Instead,
`src/analyzers/framework_recon/mod.rs` provides a bounded, non-mutating
aggregate report, and the `framework-recon` CLI subcommand serializes it as
JSON. Running `jsbeautify -d` does not implicitly run framework recon.

The two architectures should remain distinct:

- AST deobfuscation owns ordered, semantics-sensitive mutation and its
  codegen/re-parse checkpoints under `src/ast_deobfuscate/engine/`.
- Framework recon should consume source and return findings without silently
  changing the deobfuscated program.

## Existing compiled modules

| Area | Current entry point | Status / demonstrated gap |
|---|---|---|
| Angular metadata | `angular_metadata::extract_angular_metadata` | Compiled, unit-tested, and aggregated by `framework_recon`. |
| Angular templates | `angular_decompiler::decompile_angular_component` | Compiled and unit-tested; coverage is bounded by its explicit instruction parser. |
| Angular routes | `angular_route_extractor::extract_routes` | Compiled and unit-tested; primarily recognizes current documented webpack/lazy-route shapes. |
| NgRx | `ngrx_extractor::extract_ngrx_patterns` | Compiled, unit-tested, and aggregated by `framework_recon`. |
| RxJS | `rxjs_annotator::{annotate_rxjs_operators, extract_operator_usage}` | Compiled and unit-tested; operator-table/signature currency still needs framework-version fixtures. |
| LaunchDarkly | `launchdarkly_extractor::extract_feature_flags` | Compiled and unit-tested; generic typed-variation coverage exists, but broader real-bundle validation is still needed. |
| Registry / script loading | `mfe_registry_extractor::analyze_mfe_registry` | Compiled and unit-tested; name overstates scope because patterns are closer to single-spa/app registry behavior than generic Module Federation. |
| HTTP interceptors | `http_interceptor_extractor::analyze_interceptor_chain` | Compiled and unit-tested; contains application-derived assumptions and needs generic fixtures. |
| webpack runtime | `webpack_runtime_decoder::{detect_require_var, decode_webpack_runtime, annotate_webpack_runtime}` | Compiled and unit-tested; remains a text rewrite, not a registered AST pass. |
| webpack modules | `webpack_module_resolver::resolve_module_ids` | Compiled and unit-tested; package recognition is heuristic. |
| Bundle diff | `bundle_diff::{parse_chunk_manifest, diff_manifests}` | Compiled; standalone helper with no CLI. |
| Transmit SDK | `transmit_sdk_decoder::analyze_transmit_sdk` | Compiled; application/security recon helper with no CLI. |

## Current integration and remaining interface work

`framework_recon::analyze` returns a versioned `FrameworkReconReport`. The CLI
exposes it directly:

```bash
jsbeautify framework-recon bundle.js -o framework-report.json
```

Input and report sizes are bounded, findings are sorted for stable JSON, and the
analyzer does not evaluate or transform JavaScript. Remaining interface work:

1. add stdin coverage to the committed JSON output and oversized-file CLI tests;
2. add broader multi-framework fixtures and false-positive fixtures;
3. decide whether per-detector opt-in controls are needed before expanding the
   default aggregate;
4. document schema compatibility when `schema_version` changes.

## Hardening backlog

Work from evidence in the existing implementation, rather than from a fixed
framework wish list:

- rename/generalize the registry extractor before calling it Module Federation;
- validate Angular/RxJS signatures against versioned production fixtures;
- generalize LaunchDarkly and interceptor matching;
- make webpack recognition structural and test modern arrow/chunk-push output;
- add false-positive fixtures for every detector;
- add React/Next.js, Vue/Pinia, Redux/RTK Query, Vite, Svelte, Sentry, Rollup,
  Zustand, Preact, or Solid only after the dispatcher and result schema exist.

## Verification commands

```bash
# All declared targets
cargo test --all-targets

# Existing framework extractor unit tests
cargo test --lib ast_deobfuscate::angular_metadata
cargo test --lib ast_deobfuscate::angular_decompiler
cargo test --lib ast_deobfuscate::angular_route_extractor
cargo test --lib ast_deobfuscate::ngrx_extractor
cargo test --lib ast_deobfuscate::rxjs_annotator
cargo test --lib ast_deobfuscate::launchdarkly_extractor
cargo test --lib ast_deobfuscate::mfe_registry_extractor
cargo test --lib ast_deobfuscate::http_interceptor_extractor
cargo test --lib ast_deobfuscate::webpack_runtime_decoder
cargo test --lib ast_deobfuscate::webpack_module_resolver
```

These unit tests cover local extractor fixtures. The committed CLI integration
tests additionally verify JSON output, fail-closed oversized-file handling, and
that target JavaScript is not executed (`tests/integration/framework_recon.rs`).
