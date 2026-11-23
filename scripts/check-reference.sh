#!/usr/bin/env bash
# Fails if anything under reference/ was added, removed or changed.
# reference/ is read-only (CLAUDE.md). To accept a deliberate, approved update:
#   scripts/check-reference.sh --update
set -euo pipefail
cd "$(dirname "$0")/.."
manifest=docs/reference.sha256
current() { find reference -type f -not -path '*/__pycache__/*' -not -name '*.pyc' -print0 | LC_ALL=C sort -z | xargs -0 sha256sum; }
if [[ "${1:-}" == "--update" ]]; then current > "$manifest"; echo "updated $manifest"; exit 0; fi
if diff -u "$manifest" <(current); then
  echo "reference/ unchanged ($(wc -l < "$manifest") files)"
else
  echo "ERROR: reference/ differs from $manifest. reference/ is read-only (CLAUDE.md)." >&2
  exit 1
fi
