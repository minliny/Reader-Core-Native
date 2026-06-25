# ABI / Protocol / Contract / CLI Gap Checklist

Scope for this checklist:

- ABI: `include/reader_core.h`, `crates/reader-ffi`, `scripts/ffi-smoke.sh`
- Protocol: `protocol/*.schema.json`, `protocol/fixtures/conformance/**`
- Contract: `crates/reader-contract`
- CLI evidence: `tools/reader-cli -- --conformance`

Out of scope: host apps, platform bindings, rule/js/content/storage/sync crates,
and unrelated dirty worktree files.

Current scan date: 2026-06-25.

## Surface Inventory

| Surface | Current evidence | Gap status |
| --- | --- | --- |
| C ABI v1 | Public header, `reader-ffi` externs, C/C++ `ffi-smoke` symbol checks | No ABI change selected this pass |
| Command envelope | `reader-command.schema.json`, `Command`, conformance fixtures | Covered |
| Event envelope | `reader-event.schema.json`, `Event`, event deserialization tests | Covered for envelope invariants |
| Runtime config | `reader-runtime-config.schema.json`, `RuntimeConfig`, CLI config cases | Covered |
| Host bus | host request/complete/error DTOs, schema refs, CLI cases | Covered for generic host bus |

## Method Checklist

| Method | Params schema/DTO | Result schema/DTO | CLI evidence | Status |
| --- | --- | --- | --- | --- |
| `core.info` | `EmptyParams` | Generic JSON result | Capabilities assertion | Open: no typed result DTO |
| `runtime.ping` | `EmptyParams` | Generic JSON result | `pong` assertion | Open: no typed result DTO |
| `runtime.cancel` | `RuntimeCancelParams` | Generic JSON result | `cancelled` assertion | Open: no typed result DTO |
| `runtime.status` | `RuntimeStatusParams` | `RuntimeStatusData` / `RuntimeStatus` | Typed parse and negative result-shape cases | Covered |
| `runtime.shutdown` | `RuntimeShutdownParams` | `RuntimeShutdownData` | Typed parse and negative result-shape cases | Covered |
| `runtime.hostSmoke` | `HostSmokeParams` | Generic echo result | Host request/complete route cases | Covered as generic host bus |
| `host.complete` | `HostCompleteParams` | Generic host completion result | Typed params and route cases | Covered as generic host bus |
| `host.error` | `HostErrorParams` | Error event | Typed params and route cases | Covered |
| `source.import` | `SourceImportParams` | Generic JSON result | Field assertions only | Open: no typed result DTO |
| `book.search` | `BookSearchParams` | Generic JSON result | Field assertions only | Open: no typed result DTO |
| `book.detail` | `BookDetailParams` | Generic JSON result | Field assertions only | Open: no typed result DTO |
| `book.toc` | `BookTocParams` | Generic JSON result | Field assertions only | Open: no typed result DTO |
| `chapter.content` | `ChapterContentParams` | Generic JSON result | Field assertions only | Open: no typed result DTO |
| `reading.progress.update` | `ReadingProgressUpdateParams` | `ReadingProgressUpdateData` | Typed parse and negative result-shape cases | Covered in current pass |

## Shortlist

1. `reading.progress.update` result data contract. Closed in this pass.
   Runtime already emits a fixed scalar object:
   `bookId`, `chapterIndex`, `chapterOffset`, `chapterProgress`, `stored`.
   This is the smallest result-data gap to close without touching runtime,
   content, storage, or host code.
2. `runtime.cancel` result data contract.
   Result is small (`cancelled`) but belongs to runtime lifecycle rather than
   remote-reading vertical.
3. `source.import` result data contract.
   Result is small, but it would start a broader remote result DTO pass.
4. Larger remote-reading result contracts (`book.search`, `book.detail`,
   `book.toc`, `chapter.content`).
   These depend on domain object shapes and optional HTTP diagnostics, so they
   should be handled after the scalar result contracts.
