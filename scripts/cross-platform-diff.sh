#!/usr/bin/env bash
# Convenience wrapper for the cross-platform result diff tool.
#
# Compares canonical JSON outputs from iOS, Android and HarmonyOS for the same
# corpus item and reports field-level diffs (missing / extra / changed /
# type_mismatch). Platform metadata fields can be ignored via --ignore.
#
# Usage:
#   ./scripts/cross-platform-diff.sh \
#       --ios samples/tooling/cross-platform-diff/ios.json \
#       --android samples/tooling/cross-platform-diff/android.json \
#       --harmony samples/tooling/cross-platform-diff/harmony.json \
#       --ignore device timestamp runtimeVersion platform
#
# Exit codes: 0 = no differences, 1 = differences found, 2 = usage/IO/parse error.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

exec python3 "${REPO_ROOT}/tools/cross-platform-diff/cross_platform_diff.py" "$@"
