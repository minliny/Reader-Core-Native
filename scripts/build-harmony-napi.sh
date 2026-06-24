#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

artifact_sha256() {
  shasum -a 256 "$1" | awk '{print $1}'
}

artifact_bytes() {
  wc -c < "$1" | tr -d '[:space:]'
}

sdk_root="${OHOS_SDK_HOME:-}"
if [[ -z "$sdk_root" ]]; then
  echo "OHOS_SDK_HOME is not set" >&2
  exit 1
fi

native_root="$sdk_root/openharmony/native"
toolchain="$native_root/build/cmake/ohos.toolchain.cmake"
cmake_bin="$native_root/build-tools/cmake/bin/cmake"
ninja_bin="$native_root/build-tools/cmake/bin/ninja"

for required in "$toolchain" "$cmake_bin" "$ninja_bin"; do
  if [[ ! -e "$required" ]]; then
    echo "missing OHOS native build tool: $required" >&2
    exit 1
  fi
done

./scripts/build-ohos.sh

build_dir="target/harmony-napi/arm64-v8a"
"$cmake_bin" \
  -S bindings/harmony/native \
  -B "$build_dir" \
  -G Ninja \
  -DCMAKE_MAKE_PROGRAM="$ninja_bin" \
  -DCMAKE_TOOLCHAIN_FILE="$toolchain" \
  -DOHOS_ARCH=arm64-v8a \
  -DCMAKE_BUILD_TYPE=Release \
  -DREADER_CORE_NATIVE_ROOT="$PWD"

"$cmake_bin" --build "$build_dir"

output="$build_dir/libreader_core_napi.so"
test -f "$output"
echo "built $output"

reader_core_static="target/aarch64-unknown-linux-ohos/release/libreader_core.a"
symbols_file="$build_dir/libreader_core_napi.symbols.txt"
napi_symbols_file="$build_dir/libreader_core_napi.napi-symbols.txt"
nm_bin="$native_root/llvm/bin/llvm-nm"
if [[ -x "$nm_bin" ]]; then
  "$nm_bin" -D --defined-only "$output" | LC_ALL=C sort > "$symbols_file" || rm -f "$symbols_file"
  "$nm_bin" -a "$output" \
    | grep -E 'reader_core_napi|_register_reader_core_napi|napi_' \
    | LC_ALL=C sort > "$napi_symbols_file" || rm -f "$napi_symbols_file"
fi

evidence="$build_dir/harmony-napi-build-evidence.txt"
{
  echo "name=reader-core-native-harmony-napi"
  echo "target=aarch64-unknown-linux-ohos"
  echo "ohos_arch=arm64-v8a"
  echo "artifact=$output"
  echo "artifact_sha256=$(artifact_sha256 "$output")"
  echo "artifact_bytes=$(artifact_bytes "$output")"
  echo "reader_core_static=$reader_core_static"
  echo "reader_core_static_sha256=$(artifact_sha256 "$reader_core_static")"
  echo "reader_core_static_bytes=$(artifact_bytes "$reader_core_static")"
  echo "cmake=$("$cmake_bin" --version | head -n 1)"
  echo "ninja=$("$ninja_bin" --version)"
  echo "toolchain=$toolchain"
  echo "ohos_sdk_home=$sdk_root"
  echo "exports=abiVersion,createRuntime,releaseRuntime,sendCommand,cancelRequest,readEvent,pendingEventCount,completeHostRequest,failHostRequest,pingSmoke,hostSmoke"
  if [[ -f "$symbols_file" ]]; then
    echo "symbols=$symbols_file"
    echo "symbols_sha256=$(artifact_sha256 "$symbols_file")"
  else
    echo "symbols=<unavailable>"
  fi
  if [[ -f "$napi_symbols_file" ]]; then
    echo "napi_symbols=$napi_symbols_file"
    echo "napi_symbols_sha256=$(artifact_sha256 "$napi_symbols_file")"
  else
    echo "napi_symbols=<unavailable>"
  fi
} > "$evidence"
echo "evidence $evidence"
cat "$evidence"
