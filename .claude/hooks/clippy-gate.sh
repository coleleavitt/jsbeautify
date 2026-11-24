#!/usr/bin/env bash
# Stop hook: when Claude finishes a turn, if any Rust files changed in the working
# tree, run clippy.
#   - BLOCK (exit 2) only on real compile/clippy ERRORS — i.e. the crate does not
#     build. Those are unambiguous and worth fixing before the turn ends.
#   - pedantic/nursery WARNINGS are advisory: surfaced as a one-line note, never
#     blocking, so pre-existing lint debt can't trap Claude in a fix-loop.
set -uo pipefail
cd "${CLAUDE_PROJECT_DIR:-.}" || exit 0

# Fast path: nothing Rust changed -> skip the (slower) clippy compile entirely.
if ! git status --porcelain 2>/dev/null | grep -qE '\.rs$'; then
  exit 0
fi

out="$(env -u CLIPPY_CONF_DIR cargo clippy --all-targets --message-format short 2>&1)"
status=$?

if [ "$status" -ne 0 ]; then
  echo "cargo clippy failed — the crate does not build cleanly. Fix before stopping:" >&2
  printf '%s\n' "$out" | grep -E '^error|error\[' | head -40 >&2
  exit 2
fi

warns="$(printf '%s\n' "$out" | grep -cE 'warning:' || true)"
if [ "${warns:-0}" -gt 0 ]; then
  echo "i ${warns} clippy warning(s) present (pedantic/nursery). Run /gate to review."
fi
exit 0
