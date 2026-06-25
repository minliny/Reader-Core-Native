#!/usr/bin/env bash
# Wrapper for the build environment doctor.
# Resolves the worktree root and execs the Python tool with all forwarded args.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
exec python3 "$ROOT/tools/build-env-doctor/build_env_doctor.py" "$@"
