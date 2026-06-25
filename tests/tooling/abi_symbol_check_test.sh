#!/usr/bin/env bash
# tests/tooling/abi_symbol_check_test.sh
#
# End-to-end test for tools/abi-symbol-check/abi-symbol-check.py.
#
# Builds reader-ffi (static archive + cdylib) and exercises the checker against:
#   1a. static archive, default symbols        → PASS (exit 0)
#   1b. dynamic library, default symbols       → PASS (exit 0)
#   2.  bogus required symbol                  → FAIL (exit 1) + missing listed
#   3.  non-existent path                      → ERROR (exit 2)
#   4.  scripts/check-abi-symbols.sh wrapper   → forwards exit 0
#
# Run from the repo root:  bash tests/tooling/abi_symbol_check_test.sh
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/../.." && pwd)"
checker="${repo_root}/tools/abi-symbol-check/abi-symbol-check.py"

pass_count=0
fail_count=0
note() { printf '\n=== %s ===\n' "$1"; }

# run_capture <cmd...> — captures stdout+stderr into $out and exit code into $rc.
# Resets rc to 0 first so it never leaks from a previous invocation.
run_capture() {
  rc=0
  out=$("$@" 2>&1) || rc=$?
}

assert_exit() { # <expected> <label>
  if [[ "${rc}" == "$1" ]]; then
    echo "ok: $2 (exit=${rc})"
    pass_count=$((pass_count + 1))
  else
    echo "FAIL: $2 (expected exit=$1, got exit=${rc})" >&2
    fail_count=$((fail_count + 1))
  fi
}
assert_grep() { # <pattern> <label>
  if echo "${out}" | grep -q "$1"; then
    echo "ok: $2"
    pass_count=$((pass_count + 1))
  else
    echo "FAIL: $2 (pattern '$1' not in output)" >&2
    fail_count=$((fail_count + 1))
  fi
}

cd "${repo_root}"

# ---------------------------------------------------------------------------
# Build the static archive + cdylib. reader-ffi has no reader-contract
# dependency issues at this baseline, so this should build cleanly.
# ---------------------------------------------------------------------------
note "build reader-ffi (static archive + cdylib)"
cargo build -p reader-ffi --release
lib_a="target/release/libreader_core.a"
lib_dylib="target/release/libreader_core.dylib"
if [[ ! -f "${lib_a}" ]]; then
  echo "ERROR: expected static archive at ${lib_a} after build" >&2
  exit 3
fi
if [[ ! -f "${lib_dylib}" ]]; then
  echo "ERROR: expected dynamic library at ${lib_dylib} after build" >&2
  exit 3
fi

# ---------------------------------------------------------------------------
# Case 1a: static archive, default required symbols → PASS
# ---------------------------------------------------------------------------
note "case 1a: static archive, default required symbols (expect PASS)"
run_capture python3 "${checker}" "${lib_a}"
echo "${out}"
assert_exit 0 "default symbols against static archive"
assert_grep "^PASS:" "static archive PASS line"

# ---------------------------------------------------------------------------
# Case 1b: dynamic library, default required symbols → PASS
# ---------------------------------------------------------------------------
note "case 1b: dynamic library, default required symbols (expect PASS)"
run_capture python3 "${checker}" "${lib_dylib}"
echo "${out}"
assert_exit 0 "default symbols against dynamic library"
assert_grep "^PASS:" "dynamic library PASS line"

# ---------------------------------------------------------------------------
# Case 2: real library, one bogus required symbol → FAIL + missing listed
# ---------------------------------------------------------------------------
note "case 2: real library, bogus required symbol (expect FAIL)"
run_capture python3 "${checker}" "${lib_a}" --required rc_abi_version __does_not_exist__
echo "${out}"
assert_exit 1 "bogus symbol causes FAIL exit"
assert_grep "__does_not_exist__" "missing symbol listed in output"

# ---------------------------------------------------------------------------
# Case 3: non-existent path → ERROR (exit 2)
# ---------------------------------------------------------------------------
note "case 3: non-existent path (expect ERROR exit 2)"
run_capture python3 "${checker}" "/tmp/does-not-exist-${$}.a"
echo "${out}"
assert_exit 2 "missing file causes ERROR exit"
assert_grep "not found" "error message mentions 'not found'"

# ---------------------------------------------------------------------------
# Case 4: wrapper script forwards exit code
# ---------------------------------------------------------------------------
note "case 4: scripts/check-abi-symbols.sh wrapper forwards exit code"
run_capture bash "${repo_root}/scripts/check-abi-symbols.sh" "${lib_a}"
echo "${out}"
assert_exit 0 "wrapper script PASS against real library"
assert_grep "^PASS:" "wrapper PASS line"

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
note "summary"
echo "passed: ${pass_count}"
echo "failed: ${fail_count}"
if [[ "${fail_count}" -ne 0 ]]; then
  exit 1
fi
echo "ALL CHECKS PASSED"
exit 0
