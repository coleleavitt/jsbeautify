#!/usr/bin/env bash
set -euo pipefail

HOST_MANIFEST="${HOME}/.mozilla/native-messaging-hosts/mindemon_dump_host.json"
rm -f "${HOST_MANIFEST}"
echo "Removed ${HOST_MANIFEST}"
