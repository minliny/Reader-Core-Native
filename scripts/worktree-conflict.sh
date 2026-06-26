#!/usr/bin/env bash
# Wrapper for the worktree conflict checker.
# Resolves the repo root (parent of this script's directory) and execs the
# Python tool, forwarding all arguments.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

exec python3 "$ROOT/tools/worktree-conflict/worktree_conflict.py" "$@"
