#!/usr/bin/env bash
# PreToolUse hook (Edit|Write|MultiEdit): refuse edits to never-edit paths.
#   - target/, core dumps, scratch binaries -> DENY (build artifacts, not source)
# Emits a PreToolUse permission decision as JSON. No match -> allow (silent exit 0).
set -uo pipefail

input="$(cat)"
file="$(printf '%s' "$input" | jq -r '.tool_input.file_path // .tool_input.path // empty')"
[ -z "$file" ] && exit 0

emit() { # $1=decision  $2=reason
  jq -cn --arg d "$1" --arg r "$2" \
    '{hookSpecificOutput:{hookEventName:"PreToolUse",permissionDecision:$d,permissionDecisionReason:$r}}'
  exit 0
}

case "$file" in
  */target/*|*/core.[0-9]*|*.core|*/rust_out|*/test_collision)
    emit deny "$file is a build artifact, crash dump, or scratch binary — not source. Refusing the edit." ;;
esac
exit 0
