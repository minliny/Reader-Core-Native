#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

targets=(${IOS_TARGETS:-aarch64-apple-ios aarch64-apple-ios-sim})
output_dir="${IOS_XCFRAMEWORK_OUTPUT:-target/ios/ReaderCore.xcframework}"
headers_dir="target/ios/headers"

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

rm -rf "$headers_dir"
mkdir -p "$headers_dir"
cp include/reader_core.h "$headers_dir/reader_core.h"
cp bindings/ios/module.modulemap "$headers_dir/module.modulemap"

args=()
for target in "${targets[@]}"; do
  cargo rustc -p reader-ffi --release --target "$target" --lib --crate-type staticlib

  library="target/$target/release/libreader_core.a"
  test -f "$library"

  args+=("-library" "$library" "-headers" "$headers_dir")
done

rm -rf "$output_dir"
xcodebuild -create-xcframework "${args[@]}" -output "$output_dir"

test -d "$output_dir"

for header_root in "$output_dir"/*/Headers; do
  test -f "$header_root/reader_core.h"
  test -f "$header_root/module.modulemap"
done

sim_headers="$output_dir/ios-arm64-simulator/Headers"
if [[ -d "$sim_headers" ]]; then
  swift_smoke="$(mktemp -t reader-core-ios-smoke).swift"
  cat > "$swift_smoke" <<'EOF'
import ReaderCore

let abiVersion: UInt32 = rc_abi_version()
_ = abiVersion
EOF
  xcrun --sdk iphonesimulator swiftc \
    -target arm64-apple-ios-simulator \
    -I "$sim_headers" \
    -typecheck "$swift_smoke"
  rm -f "$swift_smoke"
fi

echo "built $output_dir"
