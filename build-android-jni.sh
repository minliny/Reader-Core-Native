#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

android_api="${ANDROID_API:-24}"
abis=(${ANDROID_ABIS:-arm64-v8a x86_64})
build_root="${ANDROID_JNI_BUILD_ROOT:-target/android-jni}"
libs_root="$build_root/libs"

rust_target_for_abi() {
  case "$1" in
    arm64-v8a) echo "aarch64-linux-android" ;;
    armeabi-v7a) echo "armv7-linux-androideabi" ;;
    x86_64) echo "x86_64-linux-android" ;;
    x86) echo "i686-linux-android" ;;
    *)
      echo "unsupported Android ABI: $1" >&2
      exit 1
      ;;
  esac
}

clang_prefix_for_target() {
  case "$1" in
    aarch64-linux-android) echo "aarch64-linux-android" ;;
    armv7-linux-androideabi) echo "armv7a-linux-androideabi" ;;
    x86_64-linux-android) echo "x86_64-linux-android" ;;
    i686-linux-android) echo "i686-linux-android" ;;
    *)
      echo "unsupported Rust Android target: $1" >&2
      exit 1
      ;;
  esac
}

find_ndk() {
  if [[ -n "${ANDROID_NDK_HOME:-}" && -d "${ANDROID_NDK_HOME:-}" ]] && is_complete_ndk "$ANDROID_NDK_HOME"; then
    echo "$ANDROID_NDK_HOME"
    return
  fi
  if [[ -n "${ANDROID_NDK_ROOT:-}" && -d "${ANDROID_NDK_ROOT:-}" ]] && is_complete_ndk "$ANDROID_NDK_ROOT"; then
    echo "$ANDROID_NDK_ROOT"
    return
  fi
  if [[ -n "${ANDROID_HOME:-}" && -d "$ANDROID_HOME/ndk" ]]; then
    while IFS= read -r candidate; do
      if is_complete_ndk "$candidate"; then
        echo "$candidate"
        return
      fi
    done < <(find "$ANDROID_HOME/ndk" -mindepth 1 -maxdepth 1 -type d | sort -r)
  fi
  if [[ -n "${ANDROID_SDK_ROOT:-}" && -d "$ANDROID_SDK_ROOT/ndk" ]]; then
    while IFS= read -r candidate; do
      if is_complete_ndk "$candidate"; then
        echo "$candidate"
        return
      fi
    done < <(find "$ANDROID_SDK_ROOT/ndk" -mindepth 1 -maxdepth 1 -type d | sort -r)
  fi
}

is_complete_ndk() {
  local ndk="$1"
  [[ -f "$ndk/build/cmake/android.toolchain.cmake" ]] || return 1
  [[ -d "$ndk/toolchains/llvm/prebuilt" ]] || return 1
}

find_cmake() {
  if command -v cmake >/dev/null 2>&1; then
    command -v cmake
    return
  fi
  if [[ -n "${ANDROID_HOME:-}" && -d "$ANDROID_HOME/cmake" ]]; then
    find "$ANDROID_HOME/cmake" -path "*/bin/cmake" -type f | sort -r | head -n 1
    return
  fi
  if [[ -n "${ANDROID_SDK_ROOT:-}" && -d "$ANDROID_SDK_ROOT/cmake" ]]; then
    find "$ANDROID_SDK_ROOT/cmake" -path "*/bin/cmake" -type f | sort -r | head -n 1
    return
  fi
}

find_kotlinc() {
  if command -v kotlinc >/dev/null 2>&1; then
    command -v kotlinc
    return
  fi
  local android_studio_kotlinc="/Applications/Android Studio.app/Contents/plugins/Kotlin/kotlinc/bin/kotlinc"
  if [[ -x "$android_studio_kotlinc" ]]; then
    echo "$android_studio_kotlinc"
    return
  fi
}

host_tag_for_ndk() {
  local ndk="$1"
  local prebuilt="$ndk/toolchains/llvm/prebuilt"
  local os
  local arch
  os="$(uname -s)"
  arch="$(uname -m)"

  local preferred=""
  case "$os:$arch" in
    Darwin:arm64) preferred="darwin-arm64" ;;
    Darwin:*) preferred="darwin-x86_64" ;;
    Linux:aarch64) preferred="linux-arm64" ;;
    Linux:*) preferred="linux-x86_64" ;;
  esac

  for tag in "$preferred" darwin-arm64 darwin-x86_64 linux-arm64 linux-x86_64; do
    if [[ -n "$tag" && -d "$prebuilt/$tag" ]]; then
      echo "$tag"
      return
    fi
  done
}

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "missing required command: $1" >&2
    exit 1
  fi
}

compile_java_sources() {
  if ! command -v javac >/dev/null 2>&1; then
    echo "warning: javac not found; skipped Java wrapper/sample compile check" >&2
    return
  fi

  local classes_dir="$build_root/classes"
  rm -rf "$classes_dir"
  mkdir -p "$classes_dir"

  local java_sources=()
  while IFS= read -r source; do
    java_sources+=("$source")
  done < <(find bindings/android/src/main/java bindings/android/samples/java \
    -name "*.java" | sort)

  if (( ${#java_sources[@]} == 0 )); then
    echo "missing Android Java sources" >&2
    exit 1
  fi

  if javac --help 2>&1 | grep -q -- "--release"; then
    javac --release 8 -d "$classes_dir" "${java_sources[@]}"
  else
    javac -source 8 -target 8 -d "$classes_dir" "${java_sources[@]}"
  fi

  echo "checked Android Java wrapper/sample sources"
}

compile_kotlin_samples() {
  local kotlinc_bin
  kotlinc_bin="$(find_kotlinc || true)"
  if [[ -z "$kotlinc_bin" ]]; then
    echo "warning: kotlinc not found; skipped Kotlin sample compile check" >&2
    return
  fi

  local kotlin_sources=()
  while IFS= read -r source; do
    kotlin_sources+=("$source")
  done < <(find bindings/android/samples/kotlin -name "*.kt" | sort)

  if (( ${#kotlin_sources[@]} == 0 )); then
    return
  fi

  local kotlin_classes_dir="$build_root/kotlin-classes"
  rm -rf "$kotlin_classes_dir"
  mkdir -p "$kotlin_classes_dir"
  "$kotlinc_bin" -jvm-target 1.8 \
    -classpath "$build_root/classes" \
    -d "$kotlin_classes_dir" \
    "${kotlin_sources[@]}"

  echo "checked Android Kotlin sample sources"
}

export_android_rust_env() {
  local target="$1"
  local toolchain="$2"
  local clang_prefix
  local target_var
  local target_env
  clang_prefix="$(clang_prefix_for_target "$target")"
  target_var="${target//-/_}"
  target_env="$(printf "%s" "$target" | tr "[:lower:]-" "[:upper:]_")"

  local clang="$toolchain/bin/${clang_prefix}${android_api}-clang"
  local clangxx="$toolchain/bin/${clang_prefix}${android_api}-clang++"
  local sysroot="$toolchain/sysroot"

  if [[ ! -x "$clang" || ! -x "$clangxx" ]]; then
    echo "missing Android clang for $target at API $android_api" >&2
    echo "expected: $clang" >&2
    exit 1
  fi

  export "CC_${target_var}=$clang"
  export "CXX_${target_var}=$clangxx"
  export "AR_${target_var}=$toolchain/bin/llvm-ar"
  export "RANLIB_${target_var}=$toolchain/bin/llvm-ranlib"
  export "CARGO_TARGET_${target_env}_LINKER=$clang"
  export "CFLAGS_${target_var}=--target=${clang_prefix}${android_api} --sysroot=$sysroot"
  export "CXXFLAGS_${target_var}=--target=${clang_prefix}${android_api} --sysroot=$sysroot"
  export "BINDGEN_EXTRA_CLANG_ARGS_${target_var}=--target=${clang_prefix}${android_api} --sysroot=$sysroot -I$sysroot/usr/include"
  export "BINDGEN_EXTRA_CLANG_ARGS=--target=${clang_prefix}${android_api} --sysroot=$sysroot -I$sysroot/usr/include"
}

require_command cargo
require_command rustup

compile_java_sources
compile_kotlin_samples

cmake_bin="$(find_cmake || true)"
if [[ -z "$cmake_bin" ]]; then
  echo "missing CMake" >&2
  echo "install cmake or Android SDK CMake, or put cmake on PATH" >&2
  exit 1
fi

ndk="$(find_ndk || true)"
if [[ -z "$ndk" ]]; then
  echo "missing Android NDK" >&2
  echo "set ANDROID_NDK_HOME, ANDROID_NDK_ROOT, ANDROID_HOME, or ANDROID_SDK_ROOT" >&2
  exit 1
fi

host_tag="$(host_tag_for_ndk "$ndk" || true)"
if [[ -z "$host_tag" ]]; then
  echo "could not find an NDK llvm prebuilt host under $ndk/toolchains/llvm/prebuilt" >&2
  exit 1
fi

toolchain="$ndk/toolchains/llvm/prebuilt/$host_tag"
toolchain_file="$ndk/build/cmake/android.toolchain.cmake"
if [[ ! -f "$toolchain_file" ]]; then
  echo "missing Android CMake toolchain file: $toolchain_file" >&2
  exit 1
fi

missing_targets=()
for abi in "${abis[@]}"; do
  target="$(rust_target_for_abi "$abi")"
  if ! rustup target list --installed | grep -qx "$target"; then
    missing_targets+=("$target")
  fi
done

if (( ${#missing_targets[@]} > 0 )); then
  echo "missing Rust target(s): ${missing_targets[*]}" >&2
  echo "install with: rustup target add ${missing_targets[*]}" >&2
  exit 1
fi

mkdir -p "$libs_root"

for abi in "${abis[@]}"; do
  target="$(rust_target_for_abi "$abi")"
  export_android_rust_env "$target" "$toolchain"

  cargo rustc -p reader-ffi --release --target "$target" --lib --crate-type staticlib

  static_lib="target/$target/release/libreader_core.a"
  test -f "$static_lib"

  build_dir="$build_root/cmake/$abi"
  "$cmake_bin" -S bindings/android/jni -B "$build_dir" \
    -DCMAKE_TOOLCHAIN_FILE="$toolchain_file" \
    -DANDROID_ABI="$abi" \
    -DANDROID_PLATFORM="android-$android_api" \
    -DANDROID_STL=c++_static \
    -DCMAKE_BUILD_TYPE=Release \
    -DREADER_CORE_NATIVE_ROOT="$PWD" \
    -DREADER_CORE_TARGET_TRIPLE="$target"
  "$cmake_bin" --build "$build_dir" --config Release

  output="$build_dir/libreader_core_jni.so"
  test -f "$output"
  mkdir -p "$libs_root/$abi"
  cp "$output" "$libs_root/$abi/libreader_core_jni.so"
done

echo "built Android JNI libraries under $libs_root"
