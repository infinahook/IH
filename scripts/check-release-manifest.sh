#!/usr/bin/env bash
set -euo pipefail
MANIFEST="${1:?Usage: check-release-manifest.sh path/to/latest.json}"
command -v jq >/dev/null || { echo "jq is required" >&2; exit 1; }
jq -e '.version and .notes and .platforms and (.platforms | type == "object")' "$MANIFEST" >/dev/null
echo "Manifest shape looks valid. This does NOT verify a cryptographic signature."
