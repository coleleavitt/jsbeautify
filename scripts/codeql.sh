#!/usr/bin/env bash
# Local CodeQL scan. Usage: scripts/codeql.sh [language...]   (default: rust actions)
# Databases and SARIF results go to .codeql/ (gitignored); findings are printed as level, rule, file:line.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
[ $# -gt 0 ] || set -- rust actions
mkdir -p .codeql
for lang in "$@"; do
  codeql database create ".codeql/db-$lang" --overwrite --language="$lang" --build-mode=none \
    --source-root=. --threads=0 --codescanning-config=.github/codeql/codeql-config.yml
  codeql database analyze ".codeql/db-$lang" "codeql/$lang-queries:codeql-suites/$lang-security-and-quality.qls" \
    --format=sarif-latest --output=".codeql/$lang.sarif" --threads=0
  echo "== $lang: $(jq '[.runs[].results[]] | length' ".codeql/$lang.sarif") findings (.codeql/$lang.sarif)"
  jq -r '.runs[].results[] | [(.level // "warning"), .ruleId,
    "\(.locations[0].physicalLocation.artifactLocation.uri):\(.locations[0].physicalLocation.region.startLine)"] | @tsv' \
    ".codeql/$lang.sarif"
done
