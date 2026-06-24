#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

target="${TARGET:-aarch64-unknown-linux-ohos}"

if ! rustup target list --installed | grep -qx "$target"; then
  echo "missing Rust target: $target" >&2
  echo "install it with: rustup target add $target" >&2
  exit 1
fi

cargo rustc -p reader-ffi --release --target "$target" --lib --crate-type staticlib

output="target/$target/release/libreader_core.a"
test -f "$output"
echo "built $output"
