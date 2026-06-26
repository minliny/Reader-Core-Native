#!/usr/bin/env bash
# Corpus release-gate end-to-end demo.
#
# Drives the corpus benchmark / release-gate toolchain on the committed
# four-platform fixture packs under samples/corpus-release-gate/:
#
#   real-run-collector -> benchmark-run-packager
#          |
#          +-------> release-blocker-register (gate)
#
# All generated artifacts are written under /private/tmp (never ~/Documents).
# The demo does not run Core business logic, does not run platform adapters,
# and does not declare a release ready. It only proves the corpus/diff tooling
# can repeatedly compare cli / ios / android / harmony candidate result files,
# while exposing ios / android / harmony hostParity in the collector manifest.
#
# Usage:
#   bash scripts/corpus_release_gate_demo.sh
#
# Exit code is 0 when the expected checks complete:
#   * four-platform-match has 0 mismatches;
#   * match bundle directory and zip verify with final proof + run/scenario/core binding;
#   * four-platform-mismatch blocked bundle verifies with run/scenario/core binding;
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
MATCH_RUN_ID="fixture-four-platform-match"
MISMATCH_RUN_ID="fixture-four-platform-mismatch"
SCENARIO="four-platform-search"
FIXTURE_CORE_COMMIT="090b96f"
PLATFORMS=(cli ios android harmony)

PY="python3"
COLLECT="$PY $ROOT/tools/corpus-real-run-collector/corpus_real_run_collector.py"
PACK="$PY $ROOT/tools/benchmark-run-packager/benchmark_run_packager.py"
REG="$PY $ROOT/tools/release-blocker-register/release_blocker_register.py"

collect_fixture_run() {
    local fixture="$1"
    local out="$2"
    local run_id="$3"
    local args=(
        --run-id "$run_id"
        --scenario "$SCENARIO"
        --input "$fixture/input.json"
        --source-manifest "$fixture/manifest.json"
        --canonical "$fixture/canonical-result.json"
        --out "$out"
        --register "$REGISTER"
    )
    local platform
    for platform in "${PLATFORMS[@]}"; do
        args+=(--candidate "$platform:$fixture/candidates/$platform-result.json")
    done
    $COLLECT "${args[@]}" >/dev/null
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

assert_host_parity() {
    local manifest_path="$1"
    local expected_match="$2"
    local expected_total="$3"
    "$PY" - "$manifest_path" "$expected_match" "$expected_total" <<'PY'
import json
import sys

path, expected_match, expected_total = sys.argv[1:]
expected_match = expected_match == "true"
expected_total = int(expected_total)
with open(path, "r", encoding="utf-8") as handle:
    manifest = json.load(handle)
host = manifest.get("hostParity")
if not isinstance(host, dict):
    print("missing hostParity in collector manifest", file=sys.stderr)
    sys.exit(1)
if host.get("requiredPlatforms") != ["ios", "android", "harmony"]:
    print("unexpected hostParity platforms: {0}".format(host.get("requiredPlatforms")),
          file=sys.stderr)
    sys.exit(1)
if host.get("match") is not expected_match or host.get("total") != expected_total:
    print(
        "unexpected hostParity state: match={0} total={1}".format(
            host.get("match"), host.get("total")
        ),
        file=sys.stderr,
    )
    sys.exit(1)
PY
}

assert_corpus_proof() {
    local manifest_path="$1"
    local expected_status="$2"
    "$PY" - "$manifest_path" "$expected_status" <<'PY'
import json
import sys

path, expected_status = sys.argv[1:]
with open(path, "r", encoding="utf-8") as handle:
    manifest = json.load(handle)
proof = manifest.get("corpusProof")
if not isinstance(proof, dict):
    print("missing corpusProof in collector manifest", file=sys.stderr)
    sys.exit(1)
if proof.get("type") != "corpus-same-result-proof":
    print("unexpected corpusProof type: {0}".format(proof.get("type")), file=sys.stderr)
    sys.exit(1)
if proof.get("status") != expected_status:
    print("unexpected corpusProof status: {0}".format(proof.get("status")),
          file=sys.stderr)
    sys.exit(1)
PY
}

assert_source_manifest_artifact() {
    local manifest_path="$1"
    local source_manifest_path="$2"
    "$PY" - "$manifest_path" "$source_manifest_path" <<'PY'
import hashlib
import json
import os
import sys

manifest_path, source_manifest_path = sys.argv[1:]

def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(65536), b""):
            digest.update(chunk)
    return digest.hexdigest()

with open(manifest_path, "r", encoding="utf-8") as handle:
    manifest = json.load(handle)
source_file = manifest.get("sourceManifestFile")
if not isinstance(source_file, dict):
    print("missing sourceManifestFile in collector manifest", file=sys.stderr)
    sys.exit(1)
if source_file.get("packagePath") != "raw/source-manifest.json":
    print(
        "unexpected sourceManifestFile packagePath: {0}".format(
            source_file.get("packagePath")
        ),
        file=sys.stderr,
    )
    sys.exit(1)
packaged = os.path.join(os.path.dirname(manifest_path), source_file["packagePath"])
if not os.path.isfile(packaged):
    print("missing packaged source manifest: {0}".format(packaged), file=sys.stderr)
    sys.exit(1)
if source_file.get("sourceSha256") != sha256(source_manifest_path):
    print("source manifest sourceSha256 mismatch", file=sys.stderr)
    sys.exit(1)
if source_file.get("packageSha256") != sha256(packaged):
    print("source manifest packageSha256 mismatch", file=sys.stderr)
    sys.exit(1)
PY
}

echo "== corpus release-gate demo =="
echo "work dir: $WORK"
echo

echo "[1/8] preparing four-platform match run"
collect_fixture_run "$MATCH_FIXTURE" "$MATCH_RUN" "$MATCH_RUN_ID"

echo "[2/8] asserting match diff and host parity have 0 mismatches"
assert_diff "$MATCH_RUN/diff-result.json" true 0
assert_host_parity "$MATCH_RUN/manifest.json" true 0
assert_corpus_proof "$MATCH_RUN/manifest.json" pass
assert_source_manifest_artifact "$MATCH_RUN/manifest.json" "$MATCH_FIXTURE/manifest.json"

echo "[3/8] packaging match run directory"
$PACK "$MATCH_RUN" --out "$WORK/bundle-match" --zip "$WORK/bundle-match.zip" >/dev/null
$PACK --verify-bundle "$WORK/bundle-match" \
    --require-corpus-proof-pass \
    --require-run-id "$MATCH_RUN_ID" \
    --require-scenario "$SCENARIO" \
    --require-core-commit "$FIXTURE_CORE_COMMIT" >/dev/null
$PACK --verify-bundle "$WORK/bundle-match.zip" \
    --require-corpus-proof-pass \
    --require-run-id "$MATCH_RUN_ID" \
    --require-scenario "$SCENARIO" \
    --require-core-commit "$FIXTURE_CORE_COMMIT" >/dev/null
echo "      bundle: $WORK/bundle-match"
echo "      zip: $WORK/bundle-match.zip"

echo "[4/8] preparing four-platform mismatch run"
collect_fixture_run "$MISMATCH_FIXTURE" "$MISMATCH_RUN" "$MISMATCH_RUN_ID"

echo "[5/8] asserting mismatch diff and host parity have exactly 1 mismatch"
assert_diff "$MISMATCH_RUN/diff-result.json" false 1
assert_host_parity "$MISMATCH_RUN/manifest.json" false 1
assert_corpus_proof "$MISMATCH_RUN/manifest.json" blocked
assert_source_manifest_artifact "$MISMATCH_RUN/manifest.json" "$MISMATCH_FIXTURE/manifest.json"
$PACK "$MISMATCH_RUN" --out "$WORK/bundle-mismatch" >/dev/null
$PACK --verify-bundle "$WORK/bundle-mismatch" \
    --require-run-id "$MISMATCH_RUN_ID" \
    --require-scenario "$SCENARIO" \
    --require-core-commit "$FIXTURE_CORE_COMMIT" >/dev/null
if $PACK --verify-bundle "$WORK/bundle-mismatch" \
    --require-corpus-proof-pass \
    --require-run-id "$MISMATCH_RUN_ID" \
    --require-scenario "$SCENARIO" \
    --require-core-commit "$FIXTURE_CORE_COMMIT" >/dev/null 2>&1; then
    echo "      blocked bundle unexpectedly passed final proof verification"
    exit 1
else
    echo "      final proof verification: blocked as expected"
fi
echo "      blocked bundle: $WORK/bundle-mismatch"

echo "[6/8] asserting collector registered blocker from mismatch diff"
ANDROID_BLK="$($REG --register "$REGISTER" list --status open --platform android --run-id "$MISMATCH_RUN_ID" --json | "$PY" -c 'import json,sys; d=json.load(sys.stdin); print(d[0]["id"] if d else "")')"
if [ -z "$ANDROID_BLK" ]; then
    echo "error: expected android blocker was not registered" >&2
    exit 1
fi

echo "[7/8] gate (expect blocked: android results[1].name divergence)"
if $REG --register "$REGISTER" gate --run-id "$MISMATCH_RUN_ID"; then
    echo "      gate: unexpected pass"
    exit 1
else
    echo "      gate: blocked (exit 1) as expected"
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
