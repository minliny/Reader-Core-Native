#!/usr/bin/env bash
# release-blockers wrapper.
#
# Resolves the worktree root (the parent of this script's directory) and
# execs the Python implementation, forwarding all arguments.
#
# Usage:
#   scripts/release-blockers.sh [root] [--evidence-index PATH] [--pretty] [--out PATH]
set -euo pipefail

# Directory containing this script.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# Worktree root is the parent of the scripts/ directory.
ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

exec python3 "${ROOT}/tools/release-blockers/release_blockers.py" "$@"
