# Corpus Audit Report — Round 1

- **Branch:** `codex/goal-sanitized-corpus`
- **Baseline:** `origin/codex/core-product-integration` (HEAD `fb4c3a7`)
- **Round:** 1 (initial batch)
- **Date:** 2026-06-25
- **Scope:** Establish the sanitized corpus scaffold and seed one fixture per supported source type. No code is touched on this branch.

## 1. Corpus items added this round

| ID | Type | Format | Fixture | Manifest | Primary consumer |
|----|------|--------|---------|----------|------------------|
| bs-001 | book-source | json | `fixtures/sanitized-corpus/book-source/bs-001-fixture.json` | `bs-001.manifest.json` | `codex/rule-engine-parity` |
| wp-001 | web-page | html | `fixtures/sanitized-corpus/web-page/wp-001-fixture.html` | `wp-001.manifest.json` | `codex/rule-engine-parity` |
| ja-001 | json-api | json | `fixtures/sanitized-corpus/json-api/ja-001-fixture.json` | `ja-001.manifest.json` | `codex/rule-engine-parity` |
| xf-001 | xml-feed | xml | `fixtures/sanitized-corpus/xml-feed/xf-001-fixture.xml` | `xf-001.manifest.json` | `codex/rule-engine-parity` |
| rf-001 | rss-feed | xml | `fixtures/sanitized-corpus/rss-feed/rf-001-fixture.xml` | `rf-001.manifest.json` | `codex/remote-reading-vertical` |

Each manifest records: `id`, `source_type`, `source_description` (来源类型说明), `sanitization` (脱敏说明), `capability_tags` (预期能力标签), `privacy_check` (隐私检查结果), `consumer_branch` (后续消费分支), `fixture_file`, `format`, and `added_in_round`.

## 2. Capability coverage

- **Rule engine (`codex/rule-engine-parity`):** JSONPath (`bs-001`, `ja-001`), CSS selectors + HTML entities + missing-href (`wp-001`), XML/namespace-aware parsing (`xf-001`), book-source rule sets for search/detail/toc/content (`bs-001`).
- **Remote reading (`codex/remote-reading-vertical`):** RSS feed iteration + field extraction (`rf-001`), book-source end-to-end sample (`bs-001`), JSON search API (`ja-001`).
- **Local content parsing (`codex/local-content-runtime`):** static HTML listing (`wp-001`), Atom/OPDS catalog (`xf-001`).

`ja-001` is intentionally structured to also cover future JSONPath filter expressions (`[?(@.meta.rating>4.0)]`) and slice/recursive-descent paths once those land on `codex/rule-engine-parity`.

## 3. Privacy verification

All five items passed privacy checks. For each item the manifest's `privacy_check.checked_for` list was reviewed against the fixture content:

| Check | bs-001 | wp-001 | ja-001 | xf-001 | rf-001 |
|-------|:-----:|:-----:|:-----:|:-----:|:-----:|
| Real tokens / API keys | pass | pass | pass | pass | pass |
| Cookies / auth headers | pass | pass | pass | pass | pass |
| Account credentials | pass | pass | pass | pass | pass |
| Private content | pass | pass | pass | pass | pass |
| Copyrighted long text | pass | pass | pass | pass | pass |
| Tracking scripts (HTML) | n/a | pass | n/a | n/a | n/a |

**Result:** No real tokens, cookies, accounts, private body text, or copyrighted long text are present. All hostnames use `example.test` / `img.example.test` / `feed.example.test` or relative URLs. All titles, authors, and prose are synthetic placeholders (e.g. "Sample Volume One", "Author Alpha"). Sample chapter content is two short fictional sentences.

## 4. Path / scope compliance

Allowed roots touched this round (and only these):

- `fixtures/sanitized-corpus/**` — 10 new files (5 fixtures + 5 manifests)
- `reports/corpus-audit/**` — 1 new file (this report)

Forbidden paths verified untouched:

- `tests/**` — not modified
- `crates/**` — not modified
- `protocol/**` — not modified
- `bindings/**` — not modified
- `scripts/**` — not modified
- `tools/**` — not modified
- `Cargo.*` — not modified

No code is wired up on this branch; the corpus is data-only and intended for consumption by other long-term branches.

## 5. Layout convention established

```
fixtures/sanitized-corpus/
  <source-type>/
    <id>-fixture.<ext>      # the sanitized payload
    <id>.manifest.json      # metadata required by the goal spec
reports/corpus-audit/
  <NNNN>-<slug>.md          # per-round audit report
```

IDs are zero-padded within their source type (`bs-001`, `wp-001`, ...). Round numbers are zero-padded in report filenames (`0001-...`).

## 6. Next steps (future rounds, not in this commit)

- Add a book-source fixture that exercises `@text` / `@html` pseudo-attribute selectors and `:contains(...)` once those are validated on `codex/rule-engine-parity`.
- Add a JSON fixture targeting JSONPath filter `[?(...)]` and union `[a,b]` / `[0,1]` expressions.
- Add a multi-page HTML fixture (pagination + next-link) for remote reading.
- Add a sanitized RSS-with-enclosure fixture for download/cover extraction paths.
- Each future round = one new group + one audit report + one commit, same constraints.
