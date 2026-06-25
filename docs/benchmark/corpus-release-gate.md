# Corpus benchmark & release gate

This directory documents the corpus benchmark / release-gate toolchain for
Reader-Core-Native. The toolchain establishes a **canonical output**, a
**cross-platform diff**, a **run packager**, and a **release blocker register**
with waivers, so that CLI and the three platform hosts (iOS / Android /
Harmony) can be held to the same corpus without the business implementation
being touched.

## Scope and constraints

Allowed surface (this route only touches these):

| Piece | Location |
|-------|----------|
| Canonicalizer | `scripts/corpus_canonicalize.py` |
| Cross-platform diff | `tools/cross-platform-diff/` |
| Run packager | `tools/benchmark-run-packager/` |
| Release blocker register | `tools/release-blocker-register/` |
| Sample corpus | `samples/corpus-release-gate/`, `samples/canonical/` |
| Demo script | `scripts/corpus_release_gate_demo.sh` |
| Tests | `tests/tooling/test_*.py` |

Hard constraints:

- **No business implementation changes.** The tools only read already-produced
  JSON result files; they never call into Core / Runtime / Rule / FFI.
- **No "release ready" declaration.** The register reports open-blocker
  state and a gate exit code; it never certifies a release.
- **No single-platform masquerade.** Three-platform consistency requires diff
  candidates from all three platforms. A CLI-only result cannot satisfy the
  gate for cross-platform consistency.
- **No new directories under `~/Documents`.** Run packager bundles and the
  blocker register default to `/private/tmp` and refuse to write under
  `~/Documents` (which covers the repo working tree).

## Pipeline

```
                 ┌──────────────────────┐
 platform JSON ─▶│  corpus_canonicalize │── canonical JSON ─┐
 (ios/android/   └──────────────────────┘                   │
  harmony/cli)                                                ▼
                   ┌──────────────────────┐   diff-result.json
   canonical  ───▶ │  cross_platform_diff │ ─────────────────┬──▶ benchmark_run_packager ──▶ bundle/summary.json
   reference       └──────────────────────┘                  │
                                                            └──▶ release_blocker_register ──▶ blockers (waive / close / gate)
```

### 1. Canonicalizer (`scripts/corpus_canonicalize.py`)

Normalizes a JSON result into a single comparable form so that synonymous
outputs collapse to identical bytes:

- object keys sorted recursively;
- runs of non-newline whitespace collapsed, lines stripped, blank lines trimmed;
- CRLF/CR → LF;
- HTML named/numeric entities decoded;
- a single trailing `/` stripped from URL paths;
- known run-volatile fields (`timestamp`, `request_id`, `trace_id`, …) replaced
  with a `<normalized>` sentinel.

```
python3 scripts/corpus_canonicalize.py input.json -o output.json
```

### 2. Cross-platform diff (`tools/cross-platform-diff/`)

Compares one canonical reference against N named platform candidates, after
running every side through the canonicalizer. Emits a `diff-result.json`
whose `summary` maps each candidate name to `{match, total}` — the exact
shape the run packager consumes.

```
python3 tools/cross-platform-diff/cross_platform_diff.py \
    canonical.json \
    --candidate ios:ios.json \
    --candidate android:android.json \
    --candidate harmony:harmony.json \
    -o diff-result.json
```

### 3. Run packager (`tools/benchmark-run-packager/`)

Validates a run directory (manifest + platform/canonical/diff results) and
packages it into an archivable bundle with a generated `summary.json` and
optional zip. It does not run any benchmark.

```
python3 tools/benchmark-run-packager/benchmark_run_packager.py run-dir/ --zip
```

### 4. Release blocker register (`tools/release-blocker-register/`)

A persistent JSON register of cross-platform divergences. Blockers are
derived from a `diff-result.json` (every difference of a non-matching
candidate becomes a blocker), then waived (with a mandatory rationale) or
closed. The `gate` subcommand reports how many blockers are still open and
exits non-zero when any remain — it does **not** declare a release ready.

```
python3 tools/release-blocker-register/release_blocker_register.py add-from-diff diff-result.json --run-id 2026-06-25-001
python3 tools/release-blocker-register/release_blocker_register.py waive BLK-0001 --rationale "accepted formatting drift"
python3 tools/release-blocker-register/release_blocker_register.py list --status open
python3 tools/release-blocker-register/release_blocker_register.py gate --run-id 2026-06-25-001
```

## End-to-end demo

A self-contained script drives all four pieces on the sample corpus under
`samples/corpus-release-gate/`, writing everything to `/private/tmp`:

```
bash scripts/corpus_release_gate_demo.sh
```

The demo canonicalizes a search-result corpus, diffs the canonical reference
against iOS / Android / Harmony candidates (Android diverges on one entry),
packages the run, registers the resulting blocker, shows the gate blocked,
then waives the blocker and shows the gate clear.

## Tests

All tooling tests are stdlib `unittest` and run without third-party deps:

```
python3 -m unittest discover -s tests/tooling -p 'test_*.py' -v
```

## Three-platform consistency

The gate is a per-run, per-platform open-blocker count. A run is only
eligible to be considered for cross-platform consistency when its
`diff-result.json` carries candidates from **all three** platform hosts
(ios, android, harmony). A CLI-only run can be packaged and registered, but
it cannot by itself satisfy three-platform consistency — the register will
not pretend otherwise.
