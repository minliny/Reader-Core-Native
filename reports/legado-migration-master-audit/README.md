# Legado Migration Master Audit

Date: 2026-06-25

Branch: `codex/legado-migration-master-audit`

Baseline: `origin/codex/core-product-integration` at `fb4c3a7`

This audit corrects the project direction: Reader-Core-Native is not a generic
new reading engine. It is a Legado-capability parity and Reader-Core migration
project. The target is a single native Core that can be loaded by iOS, Android,
HarmonyOS, CLI, and future hosts while preserving the behavior assets already
built in the existing Reader-Core.

## Executive Decision

The project target is:

1. Use local Legado source to define what must be compatible.
2. Use existing Reader-Core to define what can be migrated, replayed, or used as
   behavior evidence.
3. Use Reader-Core-Native plus a versioned C ABI to solve real cross-platform
   consumption.
4. Use sanitized and approved corpus benchmarks to prove that the same source,
   request chain, and chapter read flow produce the same canonical results on
   all three platforms.

Any plan that starts from "make an abstract cross-platform reading core" is the
wrong direction. Any plan that treats existing Reader-Core as disposable is also
wrong. The old Core is a behavior and evidence asset; Native Core is the
cross-platform execution vehicle.

## Source Inventory

| Source | Path | Current status | How it is used |
| --- | --- | --- | --- |
| Legado baseline | `/Users/minliny/Documents/legado` | clean `master`, head `da17bb2be` | Read-only capability baseline. No GPL code copy, translate, or rewrite. |
| Existing Reader-Core | `/Users/minliny/Documents/Reader-Core` | dirty `main`, head `cc7ae849`, many archived planning artifacts | Migration/evidence source; do not treat dirty state as stable production truth without checking the archived artifacts. |
| Native main worktree | `/Users/minliny/Documents/Reader-Core-Native` | clean when this audit branch was created; active development elsewhere | Current Rust/C ABI repo. |
| C ABI worktree | `/Users/minliny/Documents/Reader-Core-Native-c-abi-worktree` | `codex/reader-core-c-abi-stable-boundary`, clean | Completed ABI boundary work to audit and later merge. |
| Data subsystem worktree | `/Users/minliny/Documents/Reader-Core-Native-data-subsystem-storage` | `codex/data-subsystem-storage-cache-coverage`, clean | Completed data/content/storage/sync work to audit and later merge. |
| Harmony NAPI worktree | `/Users/minliny/Documents/Reader-Core-Native-harmony-napi-integration` | `codex/harmony-napi-integration`, clean, ahead 13 | Completed Harmony binding/NAPI smoke work to audit and later merge. |
| Rule/JS worktree | `/Users/minliny/Documents/Reader-Core-Native-rule-js-compat-clean` | dirty | Still active; do not integrate until committed and validated. |
| Android JNI worktree | `/Users/minliny/Documents/Reader-Core-Native/.claude/worktrees/android-jni-sdk` | `codex/android-jni-sdk`, clean, ahead 1 | Completed Android JNI first slice to audit and later merge. |
| Sanitized corpus worktree | `/Users/minliny/Documents/Reader-Core-Native/.wt-goal-sanitized-corpus` | `codex/goal-sanitized-corpus`, clean, ahead 1 | Seed corpus scaffold and first synthetic fixtures. |
| CI gates worktree | `/private/tmp/ci-gate-design-wt` | `codex/goal-ci-gate-design`, clean, ahead 2 | CI gate design only. |
| Release evidence worktree | `/private/tmp/release-evidence-wt` | `codex/goal-release-evidence`, clean, ahead 1 | Release readiness evidence pack. |
| Host contract worktree | `/private/tmp/goal-host-app-contracts-wt` | `codex/goal-host-app-contracts`, clean, ahead 2 | Host/Core responsibility contracts. |
| iOS host app | `/Users/minliny/Documents/Reader for iOS` | dirty `main` | Host-app state, not Native Core truth. Use as integration evidence only when committed. |
| Android host app | `/Users/minliny/Documents/Reader for Android` | clean `main` | Host-app integration target. |
| Harmony host app | `/Users/minliny/Documents/Reader for HarmonyOS` | dirty `codex/harmony-napi-runtime` | Host-app state, not Native Core truth. Use as integration evidence only when committed. |

## Completed Branch Merge Audit

These are not all safe to merge blindly. Some branches contain useful completed
work but also have overlapping history from earlier agent runs.

| Branch | Ahead of base | Status | Main content | Merge readiness |
| --- | ---: | --- | --- | --- |
| `codex/reader-core-runtime-protocol` | 15 | clean at audit start | Runtime status/shutdown, HTTP host completion validation, conformance fixtures; history also includes iOS/Harmony/Android changes | Needs careful integration split. Treat runtime/protocol changes as core-owned; platform files should be compared against platform branches before merge. |
| `codex/reader-core-c-abi-stable-boundary` | 15 | clean | `reader_core.h`, FFI status/last_error/panic guard, C/C++ smoke strengthening, Swift wrapper support | High value. Merge after protocol branch or replay ABI-only commits to avoid cross-branch conflicts. |
| `codex/data-subsystem-storage-cache-coverage` | 15 | clean | Storage snapshots, bookshelf queries, local book library snapshots, RSS snapshot import/export, sync package/journal, cache coverage planning | High value, but includes prior ABI/protocol carry-over. Needs path-level cherry-pick or conflict-aware merge. |
| `codex/harmony-napi-integration` | 13 | clean | Harmony NAPI wrapper, ArkTS SDK helpers, lifecycle smoke, timeout guards, smoke report artifact | Merge into Harmony lane after Core/ABI stabilizes. Does not prove HAP/device parity. |
| `codex/android-jni-sdk` | 1 | clean | JNI lifecycle, command/event bridge, host.complete, CMake build, Kotlin sample | Merge into Android lane. Still NDK/device validation dependent. |
| `codex/reader-rule-js-compat-clean` | 5 plus dirty files | active | JSONPath/CSS/JS boundary tests and compatibility work | Not merge-ready. Needs commit, `cargo test -p reader-rule -p reader-js`, and diff audit. |
| `codex/goal-ci-gate-design` | 2 | clean | `docs/ci-gates/**`, fail-closed CI gate design | Merge independently; docs-only. |
| `codex/goal-host-app-contracts` | 2 | clean | network/session and local storage/sync host contracts | Merge independently; docs-only. |
| `codex/goal-release-evidence` | 1 | clean | release readiness evidence pack | Merge independently; docs-only. |
| `codex/goal-sanitized-corpus` | 1 | clean | first sanitized corpus batch and audit report | Merge independently; data-only. |

## Documents Cleanup / Consolidation

The relevant "Documents" material is not a few root-level files. It is spread
across worktrees and archived directories:

- Existing Reader-Core archive:
  `/Users/minliny/Documents/Reader-Core/_archived_planning_2026-06-24`
- Native side worktrees:
  `/Users/minliny/Documents/Reader-Core-Native-*`
- Nested agent worktrees:
  `/Users/minliny/Documents/Reader-Core-Native/.claude/worktrees/*`
  and `/Users/minliny/Documents/Reader-Core-Native/.wt-*`
- Host app repositories:
  `Reader for iOS`, `Reader for Android`, `Reader for HarmonyOS`

This audit consolidates those materials by reference. It does not move or delete
the original files, because old Reader-Core and host app repos are dirty and
must not be modified during a Native Core audit.

## Legado Defines What Must Be Compatible

Legado read-only source areas observed:

| Capability area | Legado source paths |
| --- | --- |
| Rule parsing and rule data | `app/src/main/java/io/legado/app/model/analyzeRule/AnalyzeRule.kt`, `RuleAnalyzer.kt`, `RuleData.kt`, `RuleDataInterface.kt` |
| JSONPath / CSS / XPath / Regex | `AnalyzeByJSonPath.kt`, `AnalyzeByJSoup.kt`, `AnalyzeByXPath.kt`, `AnalyzeByRegex.kt` |
| URL/request DSL | `AnalyzeUrl.kt`, `CustomUrl.kt`, `app/src/main/java/io/legado/app/help/http/*` |
| Web book vertical | `app/src/main/java/io/legado/app/model/webBook/{SearchModel,BookList,BookInfo,BookChapterList,BookContent,WebBook}.kt` |
| Book source models | `app/src/main/java/io/legado/app/data/entities/BookSource.kt`, `BookSourcePart.kt`, source helper and debug UI paths |
| RSS | `app/src/main/java/io/legado/app/model/rss/*`, `RssSource.kt`, RSS source controllers/debug paths |
| Local book formats | `app/src/main/java/io/legado/app/model/localBook/{TextFile,EpubFile,PdfFile,MobiFile,UmdFile,LocalBook}.kt`, `modules/book/**` |
| HTTP/Cookie/session | `app/src/main/java/io/legado/app/help/http/{CookieManager,CookieStore,HttpHelper,OkHttpUtils,Cronet,BackstageWebView}.kt` |
| WebDAV/sync | `app/src/main/java/io/legado/app/lib/webdav/*` |
| Data model / Room-like persistence | `app/src/main/java/io/legado/app/data/dao/*`, `data/entities/*` |
| Web API / source web UI | `api.md`, `app/src/main/java/io/legado/app/api/**`, `modules/web/src/**` |

The compatibility ledger must therefore cover at least:

1. Rule DSL syntax and chained execution semantics.
2. JS/Rhino-like helper compatibility and host callback behavior.
3. Request DSL: method, headers, body, charset, redirect, retry, error policy.
4. Full webBook chain: search -> detail -> toc -> content -> pagination.
5. Chapter identity, ordering, duplicate detection, canonical URL stability.
6. Cookie/session/login/WebView-hosted flows.
7. RSS source import, parse, update, and reading flow.
8. Local book import, format detection, chapter/resource reads, lazy reading.
9. WebDAV backup/sync and conflict behavior.
10. Data schema, migrations, cache/progress/bookmark/download queue behavior.
11. Web API/export/import/admin surfaces.

## Existing Reader-Core Defines Migration Assets

Reader-Core archive and tests show it is not just legacy code. It already has
behavior assets that should be migrated or replayed.

Important archived facts:

- `LEGADO_FULL_CAPABILITY_MATRIX_V2_SOURCE_BACKED_SUMMARY.md`
  - 51 total capability entries.
  - 25 Core-denominator capabilities.
  - 11 host-app scope capabilities.
  - 11 product-gated capabilities.
  - `productionReady=false`, `legadoParityComplete=false`,
    `coreParityComplete=false`.
  - real corpus benchmark was not available.
- `LEGADO-COMPAT-1_CAPABILITY_GAP_MATRIX_SUMMARY.md`
  - 82 capability entries.
  - 8 supported, 35 partial, 3 missing, 10 product-approval, 11 policy-no-go,
    11 not measured.
  - 62 high-risk parity gaps.
  - top blockers are real rule-chain and corpus parity: search->detail,
    detail->toc, toc->content, chapter identity/order, duplicate chapter
    detection, canonical URL stability.
- `LEGADO-COMPAT-11_EXTERNAL_APPROVAL_ATTESTATION_RESPONSE_GAP_AUDIT_SUMMARY.md`
  - 258 follow-up response decisions missing.
  - approval captured count remained zero.
  - benchmark-ready corpus count remained zero.

Important Recovery assets:

| Recovery | What it proves in old Reader-Core | Native migration meaning |
| --- | --- | --- |
| RECOVERY-29 | JS executor, WebView DOM executor, runtime binding results | Migrate JS/runtime behavior carefully; WebView remains host adapter. |
| RECOVERY-30 | cookie jar/session/login bridge results | Native Core must own cookie/session semantics, host must supply WebView/cookie acquisition. |
| RECOVERY-31 | local book ingestion: TXT/EPUB/PDF format/encoding/chapter/resource work | Native local book should migrate behavior and tests, not restart from scratch. |
| RECOVERY-32 | local book library runtime: catalog, duplicate/change decisions, lazy chapter/resource reads, progress/cache | Native storage/content work should preserve these semantics. |
| RECOVERY-33 | unified remote/local reading runtime, offline cache, TOC refresh/update, downloads, progress remap | Native runtime/storage should target this unified behavior, not only smoke. |

Existing Reader-Core test assets observed include:

- `Tests/ReaderCoreParserTests/*`
- `Tests/ReaderCoreNetworkTests/*`
- `Tests/ReaderPlatformAdaptersTests/*`
- `Tests/ReaderCoreModelsTests/*`
- `Tests/ReaderCoreJSRendererTests/*`
- `samples/reports/latest/**`
- Apple adapters under `Adapters/Apple/**`
- HTTP adapter under `Adapters/HTTP/**`

These are migration inputs. They are not automatically production evidence for
Native Core, but they are the best source of expected behavior.

## Native Core / C ABI Solves Platform Consumption

The old Core did not become a single platform-loadable engine for all targets.
Native Core must solve that, but only after the compatibility target is clear.

The correct boundary:

| Layer | Native Core owns | Host owns |
| --- | --- | --- |
| Protocol | command/event schema, request correlation, conformance | Sending commands and consuming events |
| Runtime | lifecycle, status, shutdown, cancel, pending host operation registry | App lifecycle scheduling and threading integration |
| ABI | C ABI, error/status codes, panic guard, last_error, event payload handling | Swift/JNI/NAPI wrappers |
| Rule/request semantics | rule execution, request descriptor construction, redirect/cookie/encoding policy | Actual TLS/socket/WebView/file permission |
| Data semantics | book/source/progress/cache/download/sync models | OS sandbox directory, secure storage handle, background scheduling |
| WebView/login/captcha | host request contract and redacted session import semantics | UI interaction, WebView DOM, captcha/manual approval |

Current Native evidence:

- C ABI branch has strengthened event protocol version, command shape status,
  host event assertions, empty command rejection, cancel no-op semantics,
  last-error boundary, default config creation, and panic-guard strategy docs.
- Runtime/protocol branch has status/shutdown/cancel/host completion
  conformance work, but its history contains platform changes too.
- Android JNI branch has a first JNI lifecycle/command/event/host.complete slice.
- Harmony branch has NAPI/ArkTS SDK helpers, lifecycle smoke, timeout guards,
  native event validation, and smoke artifacts.
- Release evidence currently distinguishes Core-side smoke from App/device
  proof; Android/Harmony/iOS host-app completion remains unproven.

## Corpus Benchmark Proves Cross-Platform Behavior

Current corpus state:

- `codex/goal-sanitized-corpus` seeded 5 synthetic fixtures:
  - `bs-001` book-source JSON
  - `wp-001` static HTML page
  - `ja-001` JSON API response
  - `xf-001` XML/OPDS-style feed
  - `rf-001` RSS feed
- All have manifests with source type, sanitization, capability tags,
  privacy checks, and consumer branches.

This is a start, not parity proof. To prove "the three platforms really read
the same thing", the benchmark must evolve into:

1. Legado-defined capability corpus.
2. Reader-Core expected behavior replay corpus.
3. Native CLI canonical result corpus.
4. iOS/Android/Harmony wrapper execution corpus.
5. Cross-platform canonical DTO comparison.

Minimum benchmark result schema:

```json
{
  "caseId": "source-chain-001",
  "sourceType": "book-source",
  "capabilities": ["search", "detail", "toc", "content", "chapter-identity"],
  "expected": {
    "bookId": "...",
    "title": "...",
    "tocCount": 10,
    "chapterOrderHash": "...",
    "contentHash": "..."
  },
  "runs": {
    "cli": {"status": "pass", "hash": "..."},
    "ios": {"status": "pass", "hash": "..."},
    "android": {"status": "pass", "hash": "..."},
    "harmony": {"status": "pass", "hash": "..."}
  },
  "privacy": {
    "rawBodyPersisted": false,
    "tokensPersisted": false,
    "cookiesPersistedInReport": false
  }
}
```

Release cannot claim Legado parity until this exists for the critical paths.

## Correct Full Development Route

### Phase 0: Freeze Source-of-Truth and Branch Hygiene

Goal: prevent more planning drift.

Required outputs:

- A Legado capability ledger.
- A Reader-Core migration ledger.
- A Native branch integration ledger.
- A rule that Legado is read-only and existing Reader-Core dirty state is not
  edited by Native agents.

### Phase 1: Legado Capability Ledger

Goal: define what must be compatible before implementing more features.

Work items:

- Enumerate source-backed capability areas from Legado paths above.
- For each capability: assign Core / host / product-gated / out-of-scope.
- Attach expected evidence type: unit test, conformance fixture, corpus replay,
  host-app proof, manual approval, or policy no-go.
- Carry forward the old matrix counts but do not trust them blindly:
  `LEGADO_FULL_CAPABILITY_MATRIX_V2` is a starting point, not the final ledger.

### Phase 2: Reader-Core Migration Ledger

Goal: preserve existing behavior work.

For every old Reader-Core area:

- `migrate`: behavior/test should be ported to Native.
- `replay`: test/corpus should be used as expected output only.
- `host`: platform adapter behavior should remain outside Core.
- `archive`: not part of Native parity.

Priority migration groups:

1. Parser/rule DSL tests.
2. Network/request DSL and URLSession behavior.
3. Cookie/session redacted behavior.
4. Local book RECOVERY-31/32.
5. Unified reading runtime RECOVERY-33.
6. WebDAV and backup/sync models.

### Phase 3: Protocol and C ABI Foundation

Goal: make a single Native Core consumable.

Order:

1. Merge/replay `reader-core-runtime-protocol` core-only commits.
2. Merge/replay `reader-core-c-abi-stable-boundary` ABI-only commits.
3. Resolve schema/status/error/cancel/host operation conflicts.
4. Run:
   - `cargo test -p reader-contract -p reader-runtime -p reader-ffi`
   - `cargo run -p reader-cli -- --conformance`
   - `./scripts/ffi-smoke.sh`

Exit condition:

- CLI and C ABI can drive status, shutdown, cancel, host.request, host.complete,
  host.error, and structured errors.

### Phase 4: Rule / JS / Request Compatibility

Goal: close Legado high-risk rule-chain gaps.

Order:

1. Finish dirty `reader-rule-js-compat-clean`.
2. Add benchmark cases for JSONPath filters, slices, unions, recursive descent,
   CSS attributes/text, XPath namespaces/predicates, regex extraction/replace.
3. Add JS host callback behavior from old Reader-Core tests.
4. Define QuickJS vs host WebView split.

Exit condition:

- Rule chain search->detail->toc->content works on sanitized corpus.
- JS/WebView-only paths fail closed with host-required errors, not fake pass.

### Phase 5: Remote Reading Chain

Goal: reproduce Legado webBook vertical semantics.

Required chain:

1. source import
2. search
3. detail
4. toc
5. chapter content
6. pagination
7. cache/offline read
8. progress remap

Exit condition:

- Same corpus source yields same canonical DTO through CLI and at least one
  platform wrapper.

### Phase 6: Local Book / RSS / WebDAV / Storage

Goal: migrate old Reader-Core RECOVERY-31/32/33 and Legado non-webBook
capabilities.

Work items:

- TXT/EPUB/PDF/MOBI/UMD classification and support policy.
- RSS import/parse/update.
- storage schema, progress, cache, downloads.
- WebDAV sync and backup/restore.

Exit condition:

- Data subsystem tests pass.
- Storage snapshot/import/export behavior is deterministic.
- Unsupported local formats fail with explicit policy.

### Phase 7: Platform Real Consumption

Goal: all platforms load the same Native Core commit.

Required platform proof:

- iOS: XCFramework + Swift SDK + URLSession/WebView/session host adapters.
- Android: JNI + Kotlin/Java SDK + NDK build + App bridge smoke.
- Harmony: NAPI + ArkTS SDK + HAP/device or platform-real runner.

Exit condition:

- Each platform runs the same corpus cases and returns the same canonical hash.
- Smoke is not enough for App/device release claims.

### Phase 8: Corpus Benchmark and Release Gate

Goal: prove parity, not merely implementation volume.

Required outputs:

- authorized/sanitized corpus with privacy approvals.
- per-capability benchmark score.
- cross-platform canonical DTO comparison.
- release blocker register.
- CI gate matrix.

Exit condition:

- `coreParityComplete`, `legadoParityComplete`, and `productionReady` can only
  move after benchmark evidence and host-app proof exist.

## Immediate Integration Order

Do not merge all current branches at once. Use this order:

1. Docs/data-only branches:
   - `goal-ci-gate-design`
   - `goal-host-app-contracts`
   - `goal-release-evidence`
   - `goal-sanitized-corpus`
2. Core runtime/protocol branch, after path-level audit removes platform drift.
3. C ABI branch, after protocol shape is fixed.
4. Android JNI branch into Android lane.
5. Harmony NAPI branch into Harmony lane.
6. Data subsystem branch, with ABI/protocol conflict resolution.
7. Rule/JS branch only after dirty work is committed and validated.

## Next Missing Long-Running Goals

These are the missing goals that should exist before more feature coding:

1. `codex/goal-legado-capability-ledger`
   - output: source-backed capability ledger from local Legado.
2. `codex/goal-reader-core-migration-ledger`
   - output: migrate/replay/host/archive table for old Reader-Core assets.
3. `codex/goal-legado-rule-chain-benchmark`
   - output: search->detail->toc->content benchmark cases.
4. `codex/goal-network-session-compat`
   - output: request DSL, cookie/session, redirect/retry/charset parity.
5. `codex/goal-webdav-backup-webapi-parity`
   - output: WebDAV, backup/restore, web API parity matrix and fixtures.

## Non-Negotiable Guardrails

- Legado remains read-only.
- No GPL implementation code is copied, translated, or rewritten.
- Existing Reader-Core dirty state is not modified by Native audit agents.
- Host-app repos are evidence sources only unless a task explicitly targets that
  repo.
- Core-side smoke must never be reported as App/device parity.
- Static reports do not replace runtime or corpus proof.
- Real corpus requires approval, privacy review, and redaction.

## Current Conclusion

The Native repo now has meaningful progress in runtime/protocol, C ABI,
storage/content/sync, Harmony NAPI, Android JNI, CI design, host contracts,
release evidence, and seed corpus. However, the project is not yet aligned to a
single Legado parity ledger, and the old Reader-Core migration assets are not
yet systematically mapped into Native work.

The next engineering move should not be another generic feature branch. It
should be the two ledgers:

1. Legado capability ledger.
2. Reader-Core migration ledger.

Only after those ledgers exist should implementation branches be judged as
"closing parity" rather than "adding capability".
