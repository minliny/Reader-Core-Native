# Platform evidence samples

Synthetic, sanitized sample evidence records for the platform evidence
validator (`tools/platform-evidence-validator`). All content uses
`example.test` hosts and placeholder values — no real tokens, accounts, or
copyrighted text.

## Format

Each file is either a single evidence record or a batch envelope, conforming
to `platform-evidence/1`.

### Single record

```json
{
  "version": "platform-evidence/1",
  "platform": "ios | android | harmony",
  "kind": "smoke | build | device | corpus | unit",
  "capability": "<non-empty string, e.g. 'reader.search'>",
  "status": "pass | fail | skipped | unknown",
  "timestamp": "<ISO8601 UTC, e.g. 2026-06-25T08:00:00Z>",
  "environment": {"os": "<string>", "arch": "<string>", "toolchain": "<optional>"},
  "fixture_id": "<optional, required when kind=corpus>",
  "artifact": "<optional repo-relative path>",
  "notes": "<optional string>"
}
```

### Batch

```json
{
  "version": "platform-evidence/1",
  "records": [<record>, ...]
}
```

Inside a batch, each record may omit `version` (the envelope carries it); if
present, it must equal `platform-evidence/1`.

## Files in this directory

| File | Purpose |
| --- | --- |
| `ios_smoke.json` | Valid iOS smoke record (pass). |
| `android_corpus.json` | Valid Android corpus record with `fixture_id`. |
| `harmony_device.json` | Valid HarmonyOS device record (fail). |
| `invalid_bad_platform.json` | Deliberately invalid: bad `platform` enum. |
| `batch.json` | Valid batch envelope with two records. |

## Validating

```sh
python3 tools/platform-evidence-validator/platform_evidence_validator.py \
    samples/tooling/evidence --pretty
```

Exit code is `0` when every file is valid, `1` when any file is invalid
(this directory contains one intentionally invalid file), and `2` on
usage/IO error.
