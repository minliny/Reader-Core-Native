#!/usr/bin/env bash
# host-replay — offline replay of host.request (HTTP/Cookie/Redirect/WebView)
# responses. Dev-time only; never opens a socket, never modifies the protocol
# schema.
#
# Thin wrapper around the standalone Rust tool at tools/host-replay. Builds the
# debug binary on first use (and when the source is newer), then forwards all
# arguments to it.
#
# Usage:
#   scripts/host-replay.sh show <fixture.json> [--pretty]
#   scripts/host-replay.sh replay [--dir <dir>] [--fixture <file>] [--trace] [--update-jar <file>]
#   scripts/host-replay.sh list [--dir <dir>]
#   scripts/host-replay.sh validate <fixture.json>
#
# See samples/host-replay/FORMAT.md for the fixture format.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TOOL_DIR="$REPO_ROOT/tools/host-replay"
BIN="$TOOL_DIR/target/debug/host-replay"

# Build if missing or stale.
needs_build=0
[ -x "$BIN" ] || needs_build=1
if [ "$needs_build" -eq 0 ]; then
    newest_src="$(find "$TOOL_DIR/src" "$TOOL_DIR/Cargo.toml" -type f -exec stat -f '%m %N' {} + 2>/dev/null | sort -rn | head -1 | awk '{print $1}')"
    bin_mtime="$(stat -f '%m' "$BIN" 2>/dev/null || echo 0)"
    [ "$newest_src" -gt "$bin_mtime" ] && needs_build=1
fi
if [ "$needs_build" -eq 1 ]; then
    cargo build -q --manifest-path "$TOOL_DIR/Cargo.toml" || {
        echo "host-replay: build failed" >&2
        exit 1
    }
fi

exec "$BIN" "$@"
