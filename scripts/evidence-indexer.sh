#!/usr/bin/env bash
# evidence-indexer wrapper.
#
# Resolves the worktree root (the parent of this script's directory) and
# execs the Python implementation, forwarding all arguments.
#
# Usage:
#   scripts/evidence-indexer.sh [root] [--pretty] [--out PATH]
set -euo pipefail

# Directory containing this script.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# Worktree root is the parent of the scripts/ directory.
ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

exec python3 "${ROOT}/tools/evidence-indexer/evidence_indexer.py" "$@"
