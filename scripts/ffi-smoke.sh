#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

cargo build -p reader-ffi --release

lib=target/release/libreader_core.a

cc -Wall -Wextra -Werror -I include \
  -x c - -x none "$lib" \
  -o target/ffi-symbol-check <<'EOF'
#include "reader_core.h"

#include <stddef.h>
#include <stdint.h>

static void symbol_check_callback(void *context, const uint8_t *json,
                                  size_t json_length) {
  (void)context;
  (void)json;
  (void)json_length;
}

int main(void) {
  rc_runtime_t *runtime = 0;
  (void)rc_abi_version();
  (void)rc_last_error(0, 0);
  (void)rc_runtime_create(0, 0, symbol_check_callback, 0, &runtime);
  (void)rc_runtime_send(runtime, 0, 0);
  (void)rc_runtime_cancel(runtime, 0);
  rc_runtime_destroy(runtime);
  return 0;
}
EOF

./target/ffi-symbol-check

cc -Wall -Wextra -Werror -I include \
  tools/ffi-smoke/main.c \
  "$lib" \
  -o target/ffi-smoke-c

./target/ffi-smoke-c

c++ -std=c++17 -Wall -Wextra -Werror -I include \
  tools/ffi-smoke/main.cpp \
  "$lib" \
  -o target/ffi-smoke-cxx

./target/ffi-smoke-cxx
