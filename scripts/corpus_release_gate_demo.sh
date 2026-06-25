#!/usr/bin/env bash
# Corpus release-gate end-to-end demo.
#
# Drives the four-piece corpus benchmark / release-gate toolchain on the
# committed sample corpus under samples/corpus-release-gate/:
#
#   canonicalizer  →  cross-platform-diff  →  benchmark-run-packager
#                                                    ↓
#                                       release-blocker-register (waive / gate)
#
# All generated artifacts are written under /private/tmp (never ~/Documents).
# The demo does not run any Core business logic and does not declare a
# release ready; it only exercises the toolchain and prints the gate state.
#
# Usage:
#   bash scripts/corpus_release_gate_demo.sh
#
# Exit code reflects the FINAL gate state: 0 = no open blockers, 1 = open
# blockers remain (the demo forces the latter first, then resolves them).

set -u

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SAMPLES="$ROOT/samples/corpus-release-gate"
WORK="$(mktemp -d /private/tmp/corpus-gate-demo.XXXXXX)"
RUN_DIR="$WORK/run-android-search"
REGISTER="$WORK/blocker-register.json"
RUN_ID="demo-android-search-001"

PY="python3"
CANON="$PY $ROOT/scripts/corpus_canonicalize.py"
DIFF="$PY $ROOT/tools/cross-platform-diff/cross_platform_diff.py"
PACK="$PY $ROOT/tools/benchmark-run-packager/benchmark_run_packager.py"
REG="$PY $ROOT/tools/release-blocker-register/release_blocker_register.py"

echo "== corpus release-gate demo =="
echo "work dir: $WORK"
echo "run id:   $RUN_ID"
echo

mkdir -p "$RUN_DIR"

# 1. Canonicalize the reference and the platform outputs.
echo "[1/7] canonicalizing reference + candidates"
$CANON "$SAMPLES/canonical-search.json" -o "$RUN_DIR/canonical-result.json"
$CANON "$SAMPLES/android-search.json"   -o "$RUN_DIR/platform-result.json"

# 2. Cross-platform diff: reference vs ios / android / harmony.
echo "[2/7] cross-platform diff (ios, android, harmony)"
$DIFF "$SAMPLES/canonical-search.json" \
    --candidate "ios:$SAMPLES/ios-search.json" \
    --candidate "android:$SAMPLES/android-search.json" \
    --candidate "harmony:$SAMPLES/harmony-search.json" \
    -o "$RUN_DIR/diff-result.json"

# 3. Run manifest for the packager.
echo "[3/7] writing manifest"
cat > "$RUN_DIR/manifest.json" <<JSON
{
  "runId": "$RUN_ID",
  "scenario": "search",
  "keyword": "fox",
  "platforms": ["ios", "android", "harmony"]
}
JSON

# 4. Package the run directory into a bundle (no zip for speed).
echo "[4/7] packaging run directory"
$PACK "$RUN_DIR" --out "$WORK/bundle" >/dev/null
echo "      bundle: $WORK/bundle"

# 5. Register blockers from the diff-result.
echo "[5/7] registering blockers from diff-result"
$REG --register "$REGISTER" add-from-diff "$RUN_DIR/diff-result.json" --run-id "$RUN_ID" --severity high

# 6. Gate evaluation — should be BLOCKED (android diverges).
echo "[6/7] gate (expect blocked: android divergence)"
if $REG --register "$REGISTER" gate --run-id "$RUN_ID"; then
    echo "      gate: unexpected pass"
else
    echo "      gate: blocked (exit 1) as expected"
fi

# 7. Resolve the android blocker (waive) and re-evaluate.
ANDROID_BLK="$($REG --register "$REGISTER" list --status open --platform android --run-id "$RUN_ID" --json | $PY -c 'import sys,json; d=json.load(sys.stdin); print(d[0]["id"] if d else "")')"
echo "[7/7] waiving android blocker $ANDROID_BLK and re-evaluating gate"
$REG --register "$REGISTER" waive "$ANDROID_BLK" --rationale "accepted: android returns Lazy Cat in this fixture" --by "demo" >/dev/null

echo
echo "final gate state:"
$REG --register "$REGISTER" gate --run-id "$RUN_ID"
rc=$?
echo
echo "demo complete. final gate exit code: $rc"
exit $rc
