#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

cargo build -p reader-ffi --release

cc -I include \
  tools/ffi-smoke/main.c \
  target/release/libreader_core.a \
  -o target/ffi-smoke-c

./target/ffi-smoke-c

c++ -std=c++17 -I include \
  tools/ffi-smoke/main.cpp \
  target/release/libreader_core.a \
  -o target/ffi-smoke-cxx

./target/ffi-smoke-cxx
