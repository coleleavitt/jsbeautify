---
name: gate
description: >-
  Run the full local verification gate for jsbeautify before committing:
  rustfmt --check, clippy with pedantic/nursery as hard errors, and the
  integration test suite. Reports pass/fail per stage and exits non-zero if any
  stage fails. Use before committing or opening a PR, or when asked to "run the
  gate", "verify", or "check everything passes".
disable-model-invocation: true
---

# gate — full pre-commit verification

Run every stage, continue through failures, and report a per-stage summary. This is
the same bar CI should enforce (the repo currently has no CI, so this is the gate).

## How to run

Execute the bundled script from the repo root:

```bash
bash .claude/skills/gate/gate.sh
```

It runs, in order (continuing past failures so you see every problem at once):

1. `cargo fmt --all -- --check` — formatting matches `rustfmt.toml` (style_edition 2024)
2. `cargo clippy --all-targets -- -D warnings` — pedantic + nursery lints as **errors**
3. `cargo test` — the `integration` test target plus unit tests

Exit code is non-zero if any stage failed; the final line is `GATE PASSED ✓` or
`GATE FAILED ✗`.

## When a stage fails

- **fmt**: run `cargo fmt --all` to auto-fix, then re-run the gate.
- **clippy**: fix the lints (this crate runs `pedantic` + `nursery`; some are noisy —
  prefer fixing over `#[allow]`, and if you must allow, scope it as narrowly as possible).
- **tests**: read the failure; if it's a deobfuscation pass, consider routing the change
  through the `deobf-soundness-reviewer` subagent before re-running.

## Notes

- The default toolchain is `rustc-master` (a nightly). Run plain `cargo`; do not pass
  `+nightly`, because no `nightly` toolchain is installed.
- clippy compiles `--all-targets` (incl. tests) and may be slow on a cold cache; warm
  builds are fast.
