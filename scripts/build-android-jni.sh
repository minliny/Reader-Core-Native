#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

android_abi="${ANDROID_ABI:-arm64-v8a}"
android_api="${ANDROID_API:-23}"

case "$android_abi" in
  arm64-v8a)
    rust_target="aarch64-linux-android"
    clang_prefix="aarch64-linux-android"
    ;;
  *)
    echo "unsupported ANDROID_ABI: $android_abi" >&2
    echo "currently supported: arm64-v8a" >&2
    exit 1
    ;;
esac

ndk_root="${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}"
if [[ -z "$ndk_root" ]]; then
  sdk_root="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
  if [[ -n "$sdk_root" && -d "$sdk_root/ndk" ]]; then
    ndk_root="$(find "$sdk_root/ndk" -mindepth 1 -maxdepth 1 -type d | sort -V | tail -n 1)"
  fi
fi

if [[ -z "$ndk_root" || ! -d "$ndk_root" ]]; then
  echo "Android NDK not found." >&2
  echo "Set ANDROID_NDK_HOME or ANDROID_NDK_ROOT, or install an NDK under \$ANDROID_HOME/ndk." >&2
  echo "This build gate fails closed; Android JNI smoke is not considered passed without an NDK build." >&2
  exit 1
fi

ndk_toolchain="$ndk_root/build/cmake/android.toolchain.cmake"
if [[ ! -f "$ndk_toolchain" ]]; then
  echo "missing NDK CMake toolchain: $ndk_toolchain" >&2
  exit 1
fi

toolchain=""
for host_tag in darwin-arm64 darwin-x86_64 linux-x86_64 windows-x86_64; do
  candidate="$ndk_root/toolchains/llvm/prebuilt/$host_tag"
  if [[ -d "$candidate" ]]; then
    toolchain="$candidate"
    break
  fi
done

if [[ -z "$toolchain" ]]; then
  echo "missing Android NDK LLVM toolchain under: $ndk_root/toolchains/llvm/prebuilt" >&2
  exit 1
fi

clang="$toolchain/bin/${clang_prefix}${android_api}-clang"
clangxx="$toolchain/bin/${clang_prefix}${android_api}-clang++"
llvm_ar="$toolchain/bin/llvm-ar"
llvm_ranlib="$toolchain/bin/llvm-ranlib"
sysroot="$toolchain/sysroot"

for required in "$clang" "$clangxx" "$llvm_ar" "$llvm_ranlib" "$sysroot/usr/include/jni.h"; do
  if [[ ! -e "$required" ]]; then
    echo "missing Android NDK build input: $required" >&2
    exit 1
  fi
done

if ! command -v cmake >/dev/null 2>&1; then
  echo "cmake not found on PATH (required for the NDK build)." >&2
  echo "install CMake >= 3.18.1 (Android Studio bundles one, or use your package manager)." >&2
  exit 1
fi

if ! rustup target list --installed | grep -qx "$rust_target"; then
  echo "missing Rust target: $rust_target" >&2
  echo "install it with: rustup target add $rust_target" >&2
  exit 1
fi

target_env="${rust_target//-/_}"
target_env_upper="$(echo "$rust_target" | tr 'a-z-' 'A-Z_')"

export "CC_${target_env}=$clang"
export "CXX_${target_env}=$clangxx"
export "AR_${target_env}=$llvm_ar"
export "RANLIB_${target_env}=$llvm_ranlib"
export "CARGO_TARGET_${target_env_upper}_LINKER=$clang"
export BINDGEN_EXTRA_CLANG_ARGS="--target=${clang_prefix}${android_api} --sysroot=$sysroot -I$sysroot/usr/include"
export "BINDGEN_EXTRA_CLANG_ARGS_${target_env}=$BINDGEN_EXTRA_CLANG_ARGS"

if [[ -z "${LIBCLANG_PATH:-}" && -d /Library/Developer/CommandLineTools/usr/lib ]]; then
  export LIBCLANG_PATH=/Library/Developer/CommandLineTools/usr/lib
fi

cargo rustc -p reader-ffi --release --target "$rust_target" --lib --crate-type staticlib

staticlib="target/$rust_target/release/libreader_core.a"
if [[ ! -f "$staticlib" ]]; then
  echo "missing Reader Core static library: $staticlib" >&2
  exit 1
fi

build_dir="target/android-jni/$android_abi"
cmake_build_dir="$build_dir/cmake"
mkdir -p "$cmake_build_dir"

cmake -S bindings/android -B "$cmake_build_dir" \
  -DCMAKE_TOOLCHAIN_FILE="$ndk_toolchain" \
  -DANDROID_ABI="$android_abi" \
  -DANDROID_PLATFORM="android-$android_api" \
  -DANDROID_STL=c++_static \
  -DREADER_CORE_STATICLIB="$PWD/$staticlib" \
  -DREADER_CORE_INCLUDE_DIR="$PWD/include"

cmake --build "$cmake_build_dir" --target reader_core_jni

output="$build_dir/libreader_core_jni.so"
cp "$cmake_build_dir/libreader_core_jni.so" "$output"
test -f "$output"
echo "built $output"
