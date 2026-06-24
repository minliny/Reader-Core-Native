#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

cargo build -p reader-ffi --release

cc -I include \
  tools/ffi-smoke/main.c \
  target/release/libreader_core.a \
  -o target/ffi-smoke

./target/ffi-smoke
