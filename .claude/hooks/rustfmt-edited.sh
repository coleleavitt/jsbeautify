#!/usr/bin/env bash
# PostToolUse hook (Edit|Write|MultiEdit): format/validate the file just edited.
#   *.rs                  -> rustfmt (respects ./rustfmt.toml; style_edition 2024
#                            needs a nightly rustfmt; the default toolchain, rustc-master, is one)
#   frontend/**.{js,cjs,mjs} -> node --check (syntax validation only; never rewrites)
# Never blocks: always exits 0. Failures are advisory (shown in the transcript).
set -uo pipefail

input="$(cat)"
file="$(printf '%s' "$input" | jq -r '.tool_input.file_path // .tool_input.path // empty')"
if [ -z "$file" ] || [ ! -f "$file" ]; then
  exit 0
fi

case "$file" in
  *.rs)
    # rustfmt walks parent dirs to find rustfmt.toml on its own.
    rustfmt --edition 2024 "$file" 2>/dev/null || true
    ;;
  */frontend/*.js|*/frontend/*.cjs|*/frontend/*.mjs)
    if ! err="$(node --check "$file" 2>&1)"; then
      echo "node --check failed for $file:" >&2
      printf '%s\n' "$err" >&2
    fi
    ;;
esac
exit 0
