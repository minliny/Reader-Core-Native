#!/usr/bin/env bash
# check-abi-symbols.sh — thin wrapper around the Python ABI symbol checker.
#
# Usage:
#   scripts/check-abi-symbols.sh <library-path> [--required NAME ...]
#
# Builds nothing on its own; pass a path to an already-built static or shared
# Reader-Core library. Exits 0 on PASS, 1 on FAIL, 2 on tool error.
#
# Typical use, after `cargo build -p reader-ffi --release`:
#   scripts/check-abi-symbols.sh target/release/libreader_core.a
#   scripts/check-abi-symbols.sh target/release/libreader_core.dylib
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/.." && pwd)"
checker="${repo_root}/tools/abi-symbol-check/abi-symbol-check.py"

if [[ $# -lt 1 ]]; then
  echo "usage: $0 <library-path> [--required NAME ...]" >&2
  exit 2
fi

exec python3 "${checker}" "$@"
