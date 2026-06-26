# evidence-index samples

Synthetic, placeholder inputs for the `evidence-indexer` tool
(`tools/evidence-indexer/evidence_indexer.py`). These files exist so the
indexer can be exercised on canned data without depending on real tool
output. All content is fabricated; any hosts, versions, and paths are
placeholders.

## Tree

```
samples/tooling/evidence-index/
  README.md                              this file
  platform-evidence-batch.json           platform-evidence/1 batch (3 records)
  capability-catalog.json                capability-catalog/1 report
  build-env-doctor.json                  build-env-doctor/1 report
  reports/
    smoke-round.md                       markdown evidence (tier: smoke)
```

## Running the indexer on this sample tree

```
python3 tools/evidence-indexer/evidence_indexer.py \
    samples/tooling/evidence-index --pretty
```

When the sample tree is the scan root, `reports/smoke-round.md` has the
repo-relative path `reports/smoke-round.md`, so the markdown classifier
indexes it (tier `smoke`, platform `unknown`). When the indexer scans the
whole worktree instead, the same file's path is
`samples/tooling/evidence-index/reports/smoke-round.md`, which does not
start with a top-level `reports/` or `evidence/` directory, so it is NOT
indexed — by design, the markdown rule only covers top-level `reports/`
and `evidence/`.
