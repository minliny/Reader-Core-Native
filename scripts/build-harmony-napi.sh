#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

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
