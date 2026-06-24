#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

targets=(${IOS_TARGETS:-aarch64-apple-ios aarch64-apple-ios-sim})
output_dir="${IOS_XCFRAMEWORK_OUTPUT:-target/ios/ReaderCore.xcframework}"

missing=()
for target in "${targets[@]}"; do
  if ! rustup target list --installed | grep -qx "$target"; then
    missing+=("$target")
  fi
done

if (( ${#missing[@]} > 0 )); then
  echo "missing Rust target(s): ${missing[*]}" >&2
  echo "install with: rustup target add ${missing[*]}" >&2
  exit 1
fi

if ! command -v xcodebuild >/dev/null 2>&1; then
  echo "missing xcodebuild; install Xcode command line tools" >&2
  exit 1
fi

args=()
for target in "${targets[@]}"; do
  cargo rustc -p reader-ffi --release --target "$target" --lib --crate-type staticlib

  library="target/$target/release/libreader_core.a"
  test -f "$library"

  args+=("-library" "$library" "-headers" "include")
done

rm -rf "$output_dir"
xcodebuild -create-xcframework "${args[@]}" -output "$output_dir"

test -d "$output_dir"
echo "built $output_dir"
