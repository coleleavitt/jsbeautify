#!/usr/bin/env bash
# Full pre-commit gate for jsbeautify. Runs each stage, continues through
# failures so every problem surfaces in one pass, prints a summary, and exits
# non-zero if any stage failed.
set -uo pipefail
cd "${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel 2>/dev/null || echo .)}" || exit 1

fail=0
run() { # $1=label  $2..=command
  local label="$1"; shift
  echo "── ${label} ──────────────────────────────"
  if "$@"; then
    echo "✓ ${label}"
  else
    echo "✗ ${label}"
    fail=1
  fi
  echo
}

run "rustfmt --check"                          cargo fmt --all -- --check
run "clippy (pedantic/nursery, -D warnings)"   env -u CLIPPY_CONF_DIR cargo clippy --all-targets -- -D warnings
run "tests"                                    cargo test

if [ "${fail}" -ne 0 ]; then
  echo "GATE FAILED ✗"
  exit 1
fi
echo "GATE PASSED ✓"
