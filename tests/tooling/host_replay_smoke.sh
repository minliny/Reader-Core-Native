#!/usr/bin/env bash
# End-to-end smoke test for the host-replay tool.
#
# Builds the standalone tool, then exercises show / replay / list / validate
# against the shipped sample fixtures. Exits non-zero on any failure.
#
# Run from anywhere:
#   bash tests/tooling/host_replay_smoke.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
TOOL_DIR="$REPO_ROOT/tools/host-replay"
SAMPLES="$REPO_ROOT/samples/host-replay"
TARGET_DIR="$TOOL_DIR/target/debug"

BIN="$TARGET_DIR/host-replay"

pass=0
fail=0
assert_contains() {
    local label="$1" needle="$2" haystack="$3"
    if grep -qF -- "$needle" <<<"$haystack"; then
        pass=$((pass + 1))
        printf '  ok: %s\n' "$label"
    else
        fail=$((fail + 1))
        printf '  FAIL: %s (missing %q)\n' "$label" "$needle"
    fi
}
# Like assert_contains but with a regex (tolerates pretty-print whitespace).
assert_regex() {
    local label="$1" regex="$2" haystack="$3"
    if grep -qE -- "$regex" <<<"$haystack"; then
        pass=$((pass + 1))
        printf '  ok: %s\n' "$label"
    else
        fail=$((fail + 1))
        printf '  FAIL: %s (regex %q)\n' "$label" "$regex"
    fi
}

echo "host-replay smoke test"
echo "  building tool..."
(cargo build -q --manifest-path "$TOOL_DIR/Cargo.toml") || { echo "BUILD FAILED"; exit 1; }

echo "  validate all samples..."
for f in "$SAMPLES"/*.json; do
    # Skip co-located body files (no `format` field) — validate only real fixtures.
    if grep -q '"format": "reader-host-replay' "$f"; then
        "$BIN" validate "$f" >/dev/null || { echo "  FAIL: validate $f"; fail=$((fail + 1)); }
    fi
done
pass=$((pass + 1)); echo "  ok: validate samples"

echo "  list..."
list_out="$("$BIN" list --dir "$SAMPLES")"
assert_contains "list shows simple-get" "001-simple-get.json" "$list_out"
assert_contains "list shows wildcard" "006-wildcard-chapter.json" "$list_out"
# Co-located body file must NOT appear as a fixture.
if grep -q "004-login-response.json" <<<"$list_out"; then
    fail=$((fail + 1)); echo "  FAIL: list leaked body file"
else
    pass=$((pass + 1)); echo "  ok: list skips body file"
fi

echo "  show (simple GET)..."
show_out="$("$BIN" show "$SAMPLES/001-simple-get.json")"
assert_contains "show emits host.complete" '"method":"host.complete"' "$show_out"
assert_contains "show carries status 200" '"status":200' "$show_out"
assert_contains "show carries body" 'Dune' "$show_out"

echo "  show (error outcome)..."
err_out="$("$BIN" show "$SAMPLES/003-error-timeout.json")"
assert_contains "error emits host.error" '"method":"host.error"' "$err_out"
assert_contains "error carries code" 'HTTP_TRANSPORT_TIMEOUT' "$err_out"

echo "  show (redirect → finalUrl)..."
redir_out="$("$BIN" show "$SAMPLES/002-redirect-cookie.json")"
assert_contains "redirect emits finalUrl" '"finalUrl":"https://login.example.test/dashboard"' "$redir_out"

echo "  show (binary → bodyBase64)..."
bin_out="$("$BIN" show "$SAMPLES/005-binary-body.json")"
assert_contains "binary emits bodyBase64" '"bodyBase64"' "$bin_out"
# And must NOT emit a text body.
if grep -q '"body":' <<<"$bin_out"; then
    fail=$((fail + 1)); echo "  FAIL: binary emitted text body"
else
    pass=$((pass + 1)); echo "  ok: binary omits text body"
fi

echo "  replay (stdin → stdout, operationId correlation)..."
replay_in='{"protocolVersion":1,"requestId":100,"type":"host.request","operationId":42,"capability":"http.execute","params":{"url":"https://books.example.test/search?q=dune","method":"GET"}}'
replay_out="$(printf '%s\n' "$replay_in" | "$BIN" replay --dir "$SAMPLES" 2>/dev/null)"
assert_contains "replay emits host.complete" '"method":"host.complete"' "$replay_out"
assert_contains "replay echoes incoming operationId" '"operationId":42' "$replay_out"

echo "  replay (wildcard match)..."
wc_in='{"protocolVersion":1,"requestId":101,"type":"host.request","operationId":43,"capability":"http.execute","params":{"url":"https://content.example.test/chapters/7","method":"GET"}}'
wc_out="$(printf '%s\n' "$wc_in" | "$BIN" replay --dir "$SAMPLES" 2>/dev/null)"
assert_contains "wildcard replay matches" '"operationId":43' "$wc_out"

echo "  replay (--update-jar captures Set-Cookie)..."
jar="$(mktemp -t host-replay-jar.XXXXXX.json)"
trap 'rm -f "$jar"' EXIT
jar_in='{"protocolVersion":1,"requestId":200,"type":"host.request","operationId":50,"capability":"http.execute","params":{"url":"https://login.example.test/auth","method":"GET"}}'
printf '%s\n' "$jar_in" | "$BIN" replay --dir "$SAMPLES" --update-jar "$jar" >/dev/null 2>&1
jar_out="$(cat "$jar")"
assert_regex "jar captured sid cookie" '"name"[[:space:]]*:[[:space:]]*"sid"' "$jar_out"
assert_regex "jar captured lang cookie" '"name"[[:space:]]*:[[:space:]]*"lang"' "$jar_out"

echo
echo "results: pass=$pass fail=$fail"
if [ "$fail" -ne 0 ]; then
    echo "SMOKE TEST FAILED"
    exit 1
fi
echo "SMOKE TEST PASSED"
