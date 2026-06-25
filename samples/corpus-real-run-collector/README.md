# Corpus real-run collector sample

This sample uses the committed four-platform fixture JSON files as stand-ins
for already-produced CLI / iOS / Android / HarmonyOS run outputs. The collector
does not run Core or platform adapters; it only packages local JSON artifacts
and routes them through canonicalization, diff, and blocker registration.

```bash
python3 tools/corpus-real-run-collector/corpus_real_run_collector.py \
  --run-id sample-four-platform-match \
  --scenario fixture-search \
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
  candidates/
```

The output directory is compatible with:

```bash
python3 tools/benchmark-run-packager/benchmark_run_packager.py \
  /private/tmp/sample-four-platform-match-candidate \
  --out /private/tmp/sample-four-platform-match-bundle
```

For a mismatch example, use `samples/corpus-release-gate/four-platform-mismatch/`.
That run records the Android `results[1].name` divergence into the generated
blocker register. This is evidence collection only, not release certification.
