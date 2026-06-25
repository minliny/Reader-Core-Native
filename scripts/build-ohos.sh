#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

target="${TARGET:-aarch64-unknown-linux-ohos}"

artifact_sha256() {
  shasum -a 256 "$1" | awk '{print $1}'
}

artifact_bytes() {
  wc -c < "$1" | tr -d '[:space:]'
}

if ! rustup target list --installed | grep -qx "$target"; then
  echo "missing Rust target: $target" >&2
  echo "install it with: rustup target add $target" >&2
  exit 1
fi

# The OHOS cross-build compiles QuickJS C (via rquickjs-sys, with the `bindgen`
# feature) and therefore needs the OHOS NDK sysroot + libclang. When
# OHOS_SDK_HOME is set (same variable build-harmony-napi.sh uses), point cc-rs
# and bindgen at the bundled NDK. Without this, the build fails on
# `stdlib.h`/`stdio.h` not found. Set the variables yourself if your SDK lives
# elsewhere.
if [[ -n "${OHOS_SDK_HOME:-}" ]]; then
  native_root="$OHOS_SDK_HOME/openharmony/native"
  if [[ -d "$native_root/sysroot" && -d "$native_root/llvm/bin" ]]; then
    arch_prefix="${target%%-*}"          # aarch64, armv7, ...
    triplet_arch="${target//unknown*/}"  # aarch64-unknown-linux-ohos -> aarch64
    clang="$native_root/llvm/bin/${target}-clang"
    sysroot="$native_root/sysroot"
    export CC_${target//-/_}="$clang"
    export CXX_${target//-/_}="${clang}++"
    # Use the NDK's llvm-ar/llvm-ranlib. The host BSD `ar` on macOS silently
    # drops cross-compiled ELF object files, producing an empty libquickjs.a
    # and, downstream, undefined-symbol link errors in the harmony NAPI .so.
    export AR_${target//-/_}="$native_root/llvm/bin/llvm-ar"
    export RANLIB_${target//-/_}="$native_root/llvm/bin/llvm-ranlib"
    export CARGO_TARGET_$(echo "$target" | tr 'a-z-' 'A-Z_')_LINKER="$clang"
    export CFLAGS_${target//-/_}="--target=$target --sysroot=$sysroot"
    export CXXFLAGS_${target//-/_}="--target=$target --sysroot=$sysroot"
    # rquickjs-sys's bindgen call does not forward CFLAGS, so feed the sysroot
    # to libclang directly. Both the bare and target-suffixed names are set
    # for compatibility across bindgen versions.
    export BINDGEN_EXTRA_CLANG_ARGS="--target=$target --sysroot=$sysroot -I$sysroot/usr/include"
    export BINDGEN_EXTRA_CLANG_ARGS_${target//-/_}="--target=$target --sysroot=$sysroot -I$sysroot/usr/include"
    export LIBCLANG_PATH="${LIBCLANG_PATH:-/Library/Developer/CommandLineTools/usr/lib}"
  fi
fi

cargo rustc -p reader-ffi --release --target "$target" --lib --crate-type staticlib

output="target/$target/release/libreader_core.a"
test -f "$output"
echo "built $output"

target_env="${target//-/_}"
cc_var="CC_$target_env"
cxx_var="CXX_$target_env"
ar_var="AR_$target_env"
ranlib_var="RANLIB_$target_env"
linker_var="CARGO_TARGET_$(echo "$target" | tr 'a-z-' 'A-Z_')_LINKER"
evidence="target/$target/release/ohos-build-evidence.txt"
{
  echo "name=reader-core-native-ohos"
  echo "target=$target"
  echo "artifact=$output"
  echo "artifact_sha256=$(artifact_sha256 "$output")"
  echo "artifact_bytes=$(artifact_bytes "$output")"
  echo "rustc=$(rustc --version)"
  echo "cargo=$(cargo --version)"
  echo "ohos_sdk_home=${OHOS_SDK_HOME:-<unset>}"
  echo "$cc_var=${!cc_var:-<unset>}"
  echo "$cxx_var=${!cxx_var:-<unset>}"
  echo "$ar_var=${!ar_var:-<unset>}"
  echo "$ranlib_var=${!ranlib_var:-<unset>}"
  echo "$linker_var=${!linker_var:-<unset>}"
  echo "libclang_path=${LIBCLANG_PATH:-<unset>}"
} > "$evidence"
echo "evidence $evidence"
cat "$evidence"
