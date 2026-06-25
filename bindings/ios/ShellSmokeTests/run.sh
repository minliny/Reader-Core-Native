#!/bin/bash
# iOS host adapter ShellSmokeTests runner.
#
# Builds (if missing) the macOS host static library, compiles the Swift wrapper
# (`ReaderCoreClient.swift`) together with the host-adapter smoke
# (`host_adapter_smoke.swift`) against `target/debug/libreader_core.a`, runs the
# binary, and tees a partitioned report to `report.txt`.
#
# This is wrapper/host smoke against the macOS Core build — NOT iOS App/device
# proof. See README.md.
set -euo pipefail

cd "$(dirname "$0")/../../.."

wrapper_source="bindings/ios/Sources/ReaderCoreClient/ReaderCoreClient.swift"
smoke_source="bindings/ios/ShellSmokeTests/host_adapter_smoke.swift"
report="bindings/ios/ShellSmokeTests/report.txt"
host_lib="${READER_CORE_HOST_LIB:-${CARGO_TARGET_DIR:-target}/debug/libreader_core.a}"

if [[ ! -f "$host_lib" ]]; then
  echo "building host static library: cargo build -p reader-ffi" >&2
  cargo build -p reader-ffi
fi

tmp_dir="$(mktemp -d -t reader-core-host-adapter-smoke)"
trap 'rm -rf "$tmp_dir"' EXIT

headers="$tmp_dir/headers"
mkdir -p "$headers"
cp include/reader_core.h "$headers/reader_core.h"
cp bindings/ios/module.modulemap "$headers/module.modulemap"

bin="$tmp_dir/host-adapter-smoke"

{
  echo "# iOS host adapter ShellSmokeTest"
  echo "# date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "# host_lib: $host_lib"
  echo "# runner: bindings/ios/ShellSmokeTests/run.sh"
  echo "# note: wrapper/host smoke against macOS Core build — NOT iOS App/device proof."
  echo
} > "$report"

# swiftc writes diagnostics to stderr; capture both into the report and the terminal.
set +e
swiftc \
  -I "$headers" \
  "$wrapper_source" \
  "$smoke_source" \
  "$host_lib" \
  -o "$bin" 2>&1 | tee -a "$report"
compile_rc=${PIPESTATUS[0]}
set -e
if [[ "$compile_rc" -ne 0 ]]; then
  echo "COMPILE FAILED (exit $compile_rc)" | tee -a "$report"
  exit "$compile_rc"
fi

set +e
"$bin" 2>&1 | tee -a "$report"
run_rc=${PIPESTATUS[0]}
set -e

if [[ "$run_rc" -ne 0 ]]; then
  echo "SMOKE FAILED (exit $run_rc)" | tee -a "$report"
  exit "$run_rc"
fi

echo
echo "report written: $report"
