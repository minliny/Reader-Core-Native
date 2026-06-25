#!/usr/bin/env bash
# Wrapper for the gate declaration checker.
# Resolves the worktree root and execs the Python tool with all forwarded args.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
exec python3 "$ROOT/tools/gate-declaration/gate_declaration.py" "$@"
