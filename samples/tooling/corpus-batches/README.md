Synthetic sample manifest for the corpus-batch-selector tool.

`manifest.json` is a `fixture-manifest/1` document with 7 fixtures covering
all three priority batches:

- **P0** (2): `bs-sample-001` (book-source, synthetic), `wp-sample-001` (web-page, synthetic).
- **P1** (3): `ja-sample-001` (json-api), `xf-sample-001` (xml-feed), `rf-sample-001` (rss-feed).
- **P2** (2): `lb-sample-001` (local-book), `bs-sample-002-nonsynthetic` (book-source but `sanitization: "unknown"`).

All hosts are `example.test` / placeholder values — no real data.

Run the selector against this sample:

```
python3 tools/corpus-batch-selector/corpus_batch_selector.py \
    --manifest samples/tooling/corpus-batches/manifest.json --pretty
```
