#!/usr/bin/env bash
# Corpus release-gate end-to-end demo.
#
# Drives the corpus benchmark / release-gate toolchain on the committed
# four-platform fixture packs under samples/corpus-release-gate/:
#
#   canonicalizer  ->  cross-platform-diff  ->  benchmark-run-packager
#                                                   |
#                                                   v
#                                      release-blocker-register (gate)
#
# All generated artifacts are written under /private/tmp (never ~/Documents).
# The demo does not run Core business logic, does not run platform adapters,
# and does not declare a release ready. It only proves the corpus/diff tooling
# can repeatedly compare cli / ios / android / harmony candidate result files.
#
# Usage:
#   bash scripts/corpus_release_gate_demo.sh
#
# Exit code is 0 when the expected checks complete:
#   * four-platform-match has 0 mismatches;
#   * four-platform-mismatch creates an android blocker;
#   * the demo closes that blocker only to prove gate state can clear.

set -eu

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SAMPLES="$ROOT/samples/corpus-release-gate"
WORK="$(mktemp -d /private/tmp/corpus-gate-demo.XXXXXX)"
MATCH_FIXTURE="$SAMPLES/four-platform-match"
MISMATCH_FIXTURE="$SAMPLES/four-platform-mismatch"
MATCH_RUN="$WORK/run-four-platform-match"
MISMATCH_RUN="$WORK/run-four-platform-mismatch"
REGISTER="$WORK/blocker-register.json"
MATCH_RUN_ID="demo-four-platform-match-001"
MISMATCH_RUN_ID="demo-four-platform-mismatch-001"
PLATFORMS=(cli ios android harmony)

PY="python3"
CANON="$PY $ROOT/scripts/corpus_canonicalize.py"
DIFF="$PY $ROOT/tools/cross-platform-diff/cross_platform_diff.py"
PACK="$PY $ROOT/tools/benchmark-run-packager/benchmark_run_packager.py"
REG="$PY $ROOT/tools/release-blocker-register/release_blocker_register.py"

write_platform_pack() {
    local fixture="$1"
    local out="$2"
    "$PY" - "$fixture" "$out" "${PLATFORMS[@]}" <<'PY'
import json
import os
import sys

fixture = sys.argv[1]
out = sys.argv[2]
platforms = sys.argv[3:]
doc = {
    "type": "four-platform-candidate-result-pack",
    "fixture": os.path.basename(fixture),
    "candidates": {},
}
for platform in platforms:
    path = os.path.join(fixture, "candidates", platform + "-result.json")
    with open(path, "r", encoding="utf-8") as handle:
        doc["candidates"][platform] = json.load(handle)
with open(out, "w", encoding="utf-8") as handle:
    json.dump(doc, handle, indent=2, ensure_ascii=False)
    handle.write("\n")
PY
}

run_diff() {
    local fixture="$1"
    local out="$2"
    local args=("$fixture/canonical-result.json")
    local platform
    for platform in "${PLATFORMS[@]}"; do
        args+=(--candidate "$platform:$fixture/candidates/$platform-result.json")
    done
    $DIFF "${args[@]}" --release-gate -o "$out"
}

write_manifest() {
    local fixture="$1"
    local out="$2"
    local run_id="$3"
    "$PY" - "$fixture/manifest.json" "$out" "$run_id" <<'PY'
import json
import sys

source, out, run_id = sys.argv[1:]
with open(source, "r", encoding="utf-8") as handle:
    manifest = json.load(handle)
manifest["runId"] = run_id
manifest["generatedBy"] = "scripts/corpus_release_gate_demo.sh"
with open(out, "w", encoding="utf-8") as handle:
    json.dump(manifest, handle, indent=2, ensure_ascii=False)
    handle.write("\n")
PY
}

prepare_run_dir() {
    local fixture="$1"
    local run_dir="$2"
    local run_id="$3"

    mkdir -p "$run_dir"
    $CANON "$fixture/canonical-result.json" -o "$run_dir/canonical-result.json"
    write_platform_pack "$fixture" "$run_dir/platform-result.json"
    run_diff "$fixture" "$run_dir/diff-result.json"
    write_manifest "$fixture" "$run_dir/manifest.json" "$run_id"
}

assert_diff() {
    local diff_path="$1"
    local expected_match="$2"
    local expected_total="$3"
    "$PY" - "$diff_path" "$expected_match" "$expected_total" <<'PY'
import json
import sys

path, expected_match, expected_total = sys.argv[1:]
expected_match = expected_match == "true"
expected_total = int(expected_total)
with open(path, "r", encoding="utf-8") as handle:
    diff = json.load(handle)
if diff.get("match") is not expected_match or diff.get("total") != expected_total:
    print(
        "unexpected diff state: match={0} total={1}".format(
            diff.get("match"), diff.get("total")
        ),
        file=sys.stderr,
    )
    sys.exit(1)
missing = [p for p in ("cli", "ios", "android", "harmony")
           if p not in diff.get("candidates", {})]
if missing:
    print("missing candidate(s): " + ", ".join(missing), file=sys.stderr)
    sys.exit(1)
PY
}

echo "== corpus release-gate demo =="
echo "work dir: $WORK"
echo

echo "[1/8] preparing four-platform match run"
prepare_run_dir "$MATCH_FIXTURE" "$MATCH_RUN" "$MATCH_RUN_ID"

echo "[2/8] asserting match diff has 0 mismatches"
assert_diff "$MATCH_RUN/diff-result.json" true 0

echo "[3/8] packaging match run directory"
$PACK "$MATCH_RUN" --out "$WORK/bundle-match" >/dev/null
echo "      bundle: $WORK/bundle-match"

echo "[4/8] preparing four-platform mismatch run"
prepare_run_dir "$MISMATCH_FIXTURE" "$MISMATCH_RUN" "$MISMATCH_RUN_ID"

echo "[5/8] asserting mismatch diff has exactly 1 mismatch"
assert_diff "$MISMATCH_RUN/diff-result.json" false 1

echo "[6/8] registering blocker from mismatch diff"
$REG --register "$REGISTER" add-from-diff "$MISMATCH_RUN/diff-result.json" \
    --run-id "$MISMATCH_RUN_ID" --severity high

echo "[7/8] gate (expect blocked: android results[1].name divergence)"
if $REG --register "$REGISTER" gate --run-id "$MISMATCH_RUN_ID"; then
    echo "      gate: unexpected pass"
    exit 1
else
    echo "      gate: blocked (exit 1) as expected"
fi

ANDROID_BLK="$($REG --register "$REGISTER" list --status open --platform android --run-id "$MISMATCH_RUN_ID" --json | "$PY" -c 'import json,sys; d=json.load(sys.stdin); print(d[0]["id"] if d else "")')"
if [ -z "$ANDROID_BLK" ]; then
    echo "error: expected android blocker was not registered" >&2
    exit 1
fi

echo "[8/8] closing demo blocker $ANDROID_BLK and re-evaluating gate"
$REG --register "$REGISTER" close "$ANDROID_BLK" >/dev/null

echo
echo "final gate state:"
$REG --register "$REGISTER" gate --run-id "$MISMATCH_RUN_ID"
rc=$?
echo
echo "demo complete. final gate exit code: $rc"
exit $rc
