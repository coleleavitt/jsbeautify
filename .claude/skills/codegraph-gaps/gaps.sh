#!/usr/bin/env bash
# codegraph gap-finder: mine the agent's real tool-call history to find where it
# fell back to the shell (grep/cat/find) for STRUCTURAL code questions that
# codegraph should have answered. Output is a ranked "what is codegraph missing"
# report that prioritizes which MCP tools to build.
#
# Usage: gaps.sh [audit.jsonl] [transcript-dir]
#   audit.jsonl    default: .jfc/audit/runtime.jsonl   (jfc ToolCall log)
#   transcript-dir default: ~/.claude/projects/<sanitized-cwd>/  (Claude Code .jsonl)
set -uo pipefail

AUDIT="${1:-.jfc/audit/runtime.jsonl}"
SANITIZED="$(pwd | sed 's#/#-#g')"
TDIR="${2:-$HOME/.claude/projects/$SANITIZED}"

command -v jq >/dev/null || { echo "jq required" >&2; exit 1; }

hr() { printf '\n%s\n' "════════════════════════════════════════════════════════════════"; }

hr; echo "  CODEGRAPH GAP REPORT"; echo "  audit: $AUDIT"; echo "  transcripts: $TDIR"; hr

# ── 1. Native tool mix from Claude Code transcripts (Read/Grep/Glob/Bash/codegraph) ──
if compgen -G "$TDIR/*.jsonl" >/dev/null 2>&1; then
  echo "── Native tool usage across transcripts (the real ratio) ──"
  cat "$TDIR"/*.jsonl 2>/dev/null \
    | jq -r 'select(.type=="assistant") | .message.content[]? | select(.type=="tool_use") | .name' 2>/dev/null \
    | sed -E 's/^mcp__codegraph__.*/codegraph_*/' \
    | sort | uniq -c | sort -rn
  echo
  cg=$(cat "$TDIR"/*.jsonl 2>/dev/null | jq -r 'select(.type=="assistant") | .message.content[]? | select(.type=="tool_use") | .name' 2>/dev/null | grep -c '^mcp__codegraph__' || true)
  rd=$(cat "$TDIR"/*.jsonl 2>/dev/null | jq -r 'select(.type=="assistant") | .message.content[]? | select(.type=="tool_use") | .name' 2>/dev/null | grep -cE '^(Read|Grep|Glob)$' || true)
  echo "  codegraph_* calls: ${cg:-0}   vs   Read/Grep/Glob calls: ${rd:-0}"
  echo "  → every Read/Grep/Glob doing a STRUCTURAL lookup is a codegraph miss."
else
  echo "── (no Claude Code transcripts found at \$TDIR — skipping native-tool mix) ──"
fi

[ -f "$AUDIT" ] || { echo; echo "No audit log at $AUDIT — done."; exit 0; }

# ── 2. jfc ToolCall distribution ──
hr; echo "── jfc ToolCall distribution ──"
jq -r 'select(.kind=="ToolCall") | .subject' "$AUDIT" | sort | uniq -c | sort -rn

# ── 3. Bash classified by purpose; STRUCTURAL = codegraph-serviceable ──
hr; echo "── Bash calls by purpose (STRUCTURAL = codegraph should serve) ──"
jq -r 'select(.kind=="ToolCall" and .subject=="Bash") | .detail' "$AUDIT" \
| sed -E 's/^cd [^&]*&& *//' \
| awk '
  { c="other (actions: builds, daemons, fs writes)"
    if      ($0 ~ /grep|rg /)                            c="grep/search        → codegraph_search / codegraph_xref"
    else if ($0 ~ /\bcat |head |tail |sed |awk |less /)  c="cat/head/sed read  → codegraph_node / codegraph_explore"
    else if ($0 ~ /\bfind |\bls /)                       c="find/ls navigate   → codegraph_files / codegraph_arch"
    else if ($0 ~ /\bwc /)                               c="wc count           → codegraph_arch (metrics)"
    else if ($0 ~ /git /)                                c="git"
    else if ($0 ~ /cargo /)                              c="cargo build/test"
    else if ($0 ~ /curl |wget /)                         c="curl/http"
    count[c]++ }
  END { for (k in count) printf "%6d  %s\n", count[k], k }' \
| sort -rn

total=$(jq -r 'select(.kind=="ToolCall" and .subject=="Bash") | .detail' "$AUDIT" | wc -l)
struct=$(jq -r 'select(.kind=="ToolCall" and .subject=="Bash") | .detail' "$AUDIT" \
  | sed -E 's/^cd [^&]*&& *//' | grep -cE 'grep|rg | cat |head |tail |sed |awk |find |\bwc ' || true)
echo
awk -v s="$struct" -v t="$total" 'BEGIN{ printf "  STRUCTURAL: %d / %d Bash calls = %.0f%% the shell did codegraph'\''s job.\n", s, t, (t? 100*s/t:0) }'

# ── 4. What are we searching FOR? (codegraph_search candidates) ──
hr; echo "── Top search terms fed to grep (each = a codegraph_search/xref) ──"
jq -r 'select(.kind=="ToolCall" and .subject=="Bash") | .detail' "$AUDIT" \
| grep -oE "grep[^|]*" \
| grep -oE "[\"'][^\"']{3,40}[\"']" | tr -d "\"'" \
| sort | uniq -c | sort -rn | head -20

# ── 5. What files are we re-reading? (codegraph_node/explore candidates) ──
hr; echo "── Most-read files via shell (each = a codegraph_node/explore) ──"
jq -r 'select(.kind=="ToolCall" and .subject=="Bash") | .detail' "$AUDIT" \
| grep -oE "(cat|head|tail|sed[^ ]*|wc) [^|&;]*" \
| grep -oE "[A-Za-z0-9_./-]+\.(rs|js|toml|md|json)" \
| sort | uniq -c | sort -rn | head -20

hr
echo "VERDICT: structural shell calls map 1:1 onto codegraph tools that should"
echo "answer them. The biggest bucket with NO good codegraph equivalent is"
echo "architecture mapping (many grep+read in one burst) → build codegraph_arch."
hr