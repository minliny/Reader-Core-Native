# Corpus real-run collector sample

This sample uses the committed four-platform fixture JSON files as stand-ins
for already-produced CLI / iOS / Android / HarmonyOS run outputs. The collector
does not run Core or platform adapters; it only packages local JSON artifacts
and routes them through canonicalization, diff, and blocker registration.
CLI is the Rust/Core reference candidate; `hostParity` is the explicit
iOS / Android / HarmonyOS same-result summary.
The source manifest binds those artifacts to one declared Rust Core identity:
`businessKernel=reader-core-native-rust`, one `coreCommit`, one `abiVersion`,
and one `protocolVersion` across CLI / iOS / Android / HarmonyOS. It also
binds `schemaVersion=1`, the collector `runId/scenario`, raw SHA-256 hashes
for `input`, `canonical`, and all four candidate artifacts, expected
per-platform diff outcomes, expected host parity, and, for mismatch fixtures,
the specific blocker platform/path. Without those explicit bindings and
expected fields, a run can still be archived but `corpusProof.status` remains
`blocked`.

```bash
python3 tools/corpus-real-run-collector/corpus_real_run_collector.py \
  --run-id fixture-four-platform-match \
  --scenario four-platform-search \
  --input samples/corpus-release-gate/four-platform-match/input.json \
  --source-manifest samples/corpus-release-gate/four-platform-match/manifest.json \
  --canonical samples/corpus-release-gate/four-platform-match/canonical-result.json \
  --candidate cli:samples/corpus-release-gate/four-platform-match/candidates/cli-result.json \
  --candidate ios:samples/corpus-release-gate/four-platform-match/candidates/ios-result.json \
  --candidate android:samples/corpus-release-gate/four-platform-match/candidates/android-result.json \
  --candidate harmony:samples/corpus-release-gate/four-platform-match/candidates/harmony-result.json
```

Default output:

```text
/private/tmp/sample-four-platform-match-candidate/
  manifest.json
  platform-result.json
  canonical-result.json
  diff-result.json
  environment.json
  corpus-blocker-register.json
  input.json
  raw/
    source-manifest.json
    canonical-result.json
    cli-result.json
    ios-result.json
    android-result.json
    harmony-result.json
  candidates/
    cli-result.json
    ios-result.json
    android-result.json
    harmony-result.json
```

The output directory is compatible with:

```bash
python3 tools/benchmark-run-packager/benchmark_run_packager.py \
  /private/tmp/sample-four-platform-match-candidate \
  --out /private/tmp/sample-four-platform-match-bundle
```

The generated bundle summary includes `summary.json.evidence`, which surfaces
the declared Rust Core identity, the packaged `raw/source-manifest.json`
hashes, and each platform's raw/canonicalized result hashes for review.
`canonicalizedSha256` matches the diff-result comparison hash;
`canonicalizedFileSha256` is the package file hash. `corpusProof` summarizes
whether the local JSON corpus evidence passes the same-result gate; it is not
release certification. The packager refuses collector-style run directories
when manifest-declared package paths are missing, escape the run directory, or
no longer match their declared SHA-256 values. It also rejects bundles whose
collector manifest `diffSummary`, `hostParity`, blocker counts, or
`corpusProof.conditions` no longer match the packaged `diff-result.json` and
`corpus-blocker-register.json`. Each bundle also includes
`bundle-manifest.json` plus `bundle-manifest.sha256`; verify the checksum file
first, then use the manifest to verify every payload file in the bundle.
`--verify-bundle` also checks that `summary.json.validation.ok=true` and that
checked collector artifact/consistency validation did not fail:

```bash
python3 tools/benchmark-run-packager/benchmark_run_packager.py \
  --verify-bundle /private/tmp/sample-four-platform-match-bundle
```

Use `--require-corpus-proof-pass` when the same bundle is being accepted as the
final same-result corpus proof. Pair it with `--require-run-id`,
`--require-scenario`, and `--require-core-commit` so the proof cannot be reused
for a different corpus scenario or Rust Core commit:

```bash
python3 tools/benchmark-run-packager/benchmark_run_packager.py \
  --verify-bundle /private/tmp/sample-four-platform-match-bundle \
  --require-corpus-proof-pass \
  --require-run-id fixture-four-platform-match \
  --require-scenario four-platform-search \
  --require-core-commit "$(git rev-parse --short HEAD)"
```

For a mismatch example, use `samples/corpus-release-gate/four-platform-mismatch/`.
That run records the Android `results[1].name` divergence into the generated
blocker register. This is evidence collection only, not release certification.
