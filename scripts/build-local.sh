#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

cargo build --workspace
cargo build -p reader-ffi --release
cargo run -p reader-cli
./scripts/ffi-smoke.sh
