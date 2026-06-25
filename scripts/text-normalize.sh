#!/usr/bin/env bash
# Wrapper for the standalone text-normalize dev tool.
#
# Builds the tool in release mode on first use (or when sources change), then
# forwards all arguments to the binary. Intended for benchmark preprocessing
# pipelines: `./scripts/text-normalize.sh --lenient --hash-only chapter.txt`.
#
# This is a development-time convenience and is NOT wired into the main
# workspace build — the tool lives as a standalone crate under
# tools/text-normalize and is not a workspace member.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TOOL_DIR="$REPO_ROOT/tools/text-normalize"
BIN="$TOOL_DIR/target/release/text-normalize"

needs_build=0
if [[ ! -x "$BIN" ]]; then
  needs_build=1
elif [[ "$TOOL_DIR/src/lib.rs" -nt "$BIN" ]]; then
  needs_build=1
elif [[ "$TOOL_DIR/src/main.rs" -nt "$BIN" ]]; then
  needs_build=1
fi

if [[ "$needs_build" -eq 1 ]]; then
  echo "text-normalize: building release binary..." >&2
  cargo build --release --manifest-path "$TOOL_DIR/Cargo.toml" >&2
fi

exec "$BIN" "$@"
