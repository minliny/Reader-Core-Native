#!/usr/bin/env bash
# Repo-root wrapper for the fixture-manifest generator.
# Usage: scripts/fixture-manifest.sh <root> [--include NAME ...] [--indent N] [--pretty]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

exec python3 "${ROOT}/tools/fixture-manifest/fixture_manifest.py" "$@"
