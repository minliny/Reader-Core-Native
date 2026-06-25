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
| Stateful runtime protocol | active `requestId` registry, cancel idempotency, shutdown latch | Duplicate active `requestId` CLI evidence selected this pass |

## Full Scan Matrix

| Area | Protocol / ABI source | Contract / CLI / smoke evidence | Scan result |
| --- | --- | --- | --- |
| C ABI version and status codes | `include/reader_core.h` enums and `rc_abi_version` | `crates/reader-ffi`, C/C++ `ffi-smoke` static asserts | Covered |
| C ABI last-error slot | `rc_last_error` docs and error enum | `reader-ffi` tests, C/C++ `ffi-smoke` null/zero-capacity/clear cases | Covered |
| C ABI lifecycle | `rc_runtime_create`, `rc_runtime_destroy` docs | `reader-ffi` tests, C/C++ create failure/default config/destroy cases | Covered |
| C ABI send envelope failures | `rc_runtime_send` docs | `reader-ffi` tests, C/C++ malformed JSON, protocolVersion, requestId, method, params cases | Covered |
| C ABI cancel semantics | `rc_runtime_cancel` docs | `reader-ffi` tests, C/C++ missing/completed/pending cancel cases | Covered |
| Command schema methods | `reader-command.schema.json` examples and method refs | `Command`, params DTO tests, CLI valid/invalid fixtures | Covered |
| Event schema result data | `reader-event.schema.json` `$defs/*Data` | Contract DTO parse/reject tests, CLI typed result parse/reject cases | Covered |
| Runtime config schema | `reader-runtime-config.schema.json` | `RuntimeConfig`, CLI valid/invalid config cases, C/C++ create config failures | Covered |
| Host bus protocol | host request/complete/error schema and DTOs | Contract host DTO tests, CLI routing/error cases, C/C++ host bus smoke | Covered |
| Remote HTTP continuation | `HostHttpRequest` / `HostHttpResponse` defs | Contract DTO tests, CLI metadata/error cases, C/C++ http.execute smoke | Covered |
| Runtime status/shutdown | `RuntimeStatusData`, `RuntimeShutdownData` | Contract DTO tests, CLI lifecycle cases, C/C++ shutdown/status smoke | Covered |
| Stateful duplicate `requestId` | `rc_runtime_send` protocol-error doc and runtime active registry | ABI/Rust/C evidence existed; CLI conformance fixture was missing | Closed in this pass |

## Method Checklist

| Method | Params schema/DTO | Result schema/DTO | CLI evidence | Status |
| --- | --- | --- | --- | --- |
| `core.info` | `EmptyParams` | `CoreInfoData` | Typed parse and negative result-shape cases | Covered in current pass |
| `runtime.ping` | `EmptyParams` | `RuntimePingData` | Typed parse and negative result-shape cases | Covered in current pass |
| `runtime.cancel` | `RuntimeCancelParams` | `RuntimeCancelData` | Typed parse and negative result-shape cases | Covered in current pass |
| `runtime.status` | `RuntimeStatusParams` | `RuntimeStatusData` / `RuntimeStatus` | Typed parse and negative result-shape cases | Covered |
| `runtime.shutdown` | `RuntimeShutdownParams` | `RuntimeShutdownData` | Typed parse and negative result-shape cases | Covered |
| `runtime.hostSmoke` | `HostSmokeParams` | Generic echo result | Host request/complete route cases | Covered as generic host bus |
| `host.complete` | `HostCompleteParams` | Generic host completion result | Typed params and route cases | Covered as generic host bus |
| `host.error` | `HostErrorParams` | Error event | Typed params and route cases | Covered |
| `source.import` | `SourceImportParams` | `SourceImportData` | Typed parse and negative result-shape cases | Covered in current pass |
| `book.search` | `BookSearchParams` | `BookSearchData` | Typed parse and negative result-shape cases | Covered in current pass |
| `book.detail` | `BookDetailParams` | `BookDetailData` | Typed parse and negative result-shape cases | Covered in current pass |
| `book.toc` | `BookTocParams` | `BookTocData` | Typed parse and negative result-shape cases | Covered in current pass |
| `chapter.content` | `ChapterContentParams` | `ChapterContentData` | Typed parse and negative result-shape cases | Covered in current pass |
| `reading.progress.update` | `ReadingProgressUpdateParams` | `ReadingProgressUpdateData` | Typed parse and negative result-shape cases | Covered in current pass |

## Shortlist

1. `reading.progress.update` result data contract. Closed in this pass.
   Runtime already emits a fixed scalar object:
   `bookId`, `chapterIndex`, `chapterOffset`, `chapterProgress`, `stored`.
   This is the smallest result-data gap to close without touching runtime,
   content, storage, or host code.
2. `runtime.cancel` result data contract. Closed in this pass.
   Result is small (`cancelled`) but belongs to runtime lifecycle rather than
   remote-reading vertical.
3. `runtime.ping` result data contract. Closed in this pass.
   Result is small (`pong`, `method`) and closes another runtime-control
   result object without touching runtime, ABI, or host code.
4. `core.info` result data contract. Closed in this pass.
   Result is larger because it binds capability advertisement and version
   fields, but it remains a Core-owned protocol contract.
5. `source.import` result data contract. Closed in this pass.
   Result is small, but it would start a broader remote result DTO pass.
6. `book.search` result data contract. Closed in this pass.
   The result now has a typed top-level `sourceId`/`books` contract, a minimal
   stable book item shape, and optional host HTTP diagnostics.
7. `book.detail` result data contract. Closed in this pass.
   The result now has a typed top-level `sourceId`/`book` contract, a stable
   book detail object shape, and optional host HTTP diagnostics.
8. `book.toc` result data contract. Closed in this pass.
   The result now has a typed top-level `sourceId`/`bookId`/`toc` contract,
   stable TOC entry shape, and optional host HTTP diagnostics.
9. `chapter.content` result data contract. Closed in this pass.
   The result now has a typed top-level `sourceId`/`bookId`/`chapterTitle`/
   `content`/`via` contract, accepts JS JSON output, and carries optional host
   HTTP diagnostics.
10. Duplicate active `requestId` stateful protocol evidence. Closed in this
    pass. The invariant already existed in ABI docs and Rust/C smoke, but the
    CLI conformance suite now has a protocol fixture proving the duplicate send
    is rejected synchronously with `INVALID_MESSAGE` and emits no async event.
