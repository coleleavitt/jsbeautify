#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HOST_DIR="${HOME}/.mozilla/native-messaging-hosts"
HOST_MANIFEST="${HOST_DIR}/mindemon_dump_host.json"
HOST_PATH="${ROOT_DIR}/native/mindemon_host.py"

mkdir -p "${HOST_DIR}"
chmod +x "${HOST_PATH}"

cat > "${HOST_MANIFEST}" <<JSON
{
  "name": "mindemon_dump_host",
  "description": "Mindemon local /tmp dump writer",
  "path": "${HOST_PATH}",
  "type": "stdio",
  "allowed_extensions": [
    "mindemon-runtime-dumper@local"
  ]
}
JSON

echo "Installed Firefox native messaging host:"
echo "${HOST_MANIFEST}"
echo
echo "Dumps will be written under /tmp/mindemon-dumps/"
