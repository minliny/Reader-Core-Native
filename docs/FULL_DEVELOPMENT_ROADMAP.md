# Reader-Core-Native Full Development Roadmap

Date: 2026-06-25

Branch baseline: `codex/full-branch-directory-consolidation`

This is the full development target and route after the branch/worktree
consolidation. It corrects the project direction: Reader-Core-Native is not a
generic reading engine. It is a Native rebuild that must match Legado capability
semantics, preserve old Reader-Core behavior assets, expose a stable Native
Core/C ABI for every host platform, and prove identical reading results through
corpus benchmarks.

## Final Target

Build one Rust Native Core that can be loaded by iOS, Android, HarmonyOS, CLI,
and future hosts through a versioned C ABI, while matching the capability
surface defined by the local Legado source and migrating/replaying the useful
behavior work already present in the old Reader-Core.

The project is complete only when the same approved corpus case can run through
CLI, iOS, Android, and HarmonyOS wrappers and produce the same canonical book,
TOC, chapter, progress, and content hashes for the critical reading flows.

## Source Of Truth Order

1. **Legado defines what must be compatible.**
   Use `/Users/minliny/Documents/legado` as a read-only source-backed capability
   baseline. Do not copy, translate, or rewrite GPL implementation code.
2. **Old Reader-Core defines what already exists and how it migrates.**
   Use `/Users/minliny/Documents/Reader-Core` as a read-only behavior, test,
   fixture, and evidence source. Classify every useful asset as migrate,
   replay, host, or archive.
3. **Reader-Core-Native plus C ABI defines how every platform consumes Core.**
   Core owns deterministic reading semantics and protocol/ABI contracts. Host
   apps own platform transport, WebView, secure storage, permissions, UI, and
   packaging.
4. **Corpus benchmark proves the claim.**
   No parity or production claim is accepted without canonical corpus evidence
   across CLI and the three platform wrappers.

## Non-Negotiable Guardrails

- Legado is read-only.
- No GPL implementation code from Legado is copied, translated, or rewritten.
- Old Reader-Core is read-only during Native work unless a task explicitly
  targets that repository.
- Host app repositories are integration evidence sources unless a task
  explicitly targets that host repo.
- Core-side smoke is not App/device proof.
- Static reports do not replace runtime or corpus evidence.
- Real corpus requires approval, privacy review, and redaction.
- A branch can add infrastructure without corpus proof, but it cannot be
  counted as Legado parity closure until corpus proof exists.

## Current Consolidated Baseline

The current Native integration branch has already consolidated the previously
scattered workstreams:

- runtime/protocol and conformance fixtures
- stable C ABI boundary and C/C++ smoke
- Android JNI and Java/Kotlin wrapper shape
- Harmony NAPI and ArkTS wrapper shape
- iOS XCFramework/Swift wrapper smoke
- data subsystem: storage, local TXT book, RSS, sync packages/journal
- rule/JS compatibility improvements
- host app contracts, CI gate design, release evidence, and seed corpus

The current root Native worktree is:

```text
/Users/minliny/Documents/Reader-Core-Native
```

External directories remain intentionally separate inputs:

| Path | Role |
| --- | --- |
| `/Users/minliny/Documents/legado` | Read-only compatibility baseline |
| `/Users/minliny/Documents/Reader-Core` | Old Core migration/evidence source |
| `/Users/minliny/Documents/Reader for iOS` | iOS host-app integration target/evidence |
| `/Users/minliny/Documents/Reader for Android` | Android host-app integration target/evidence |
| `/Users/minliny/Documents/Reader for HarmonyOS` | HarmonyOS host-app integration target/evidence |
| `/Users/minliny/Documents/Reader UI` | UI/design source, not Core truth |

## Compatibility Scope

The Legado capability ledger must cover at least these areas:

| Area | Core/host decision to make |
| --- | --- |
| Rule DSL and chained execution | Core owns deterministic selectors, fallback, chaining, transforms, and errors |
| JSONPath/CSS/XPath/Regex | Core owns compatible extraction behavior and edge cases |
| JS helper/runtime behavior | Core owns sandboxed deterministic JS; WebView-only behavior becomes host-required |
| URL/request DSL | Core owns request descriptors and policy; host executes network/WebView |
| Web book chain | Core owns source import, search, detail, TOC, content, pagination, identity, cache, progress |
| Cookie/session/login | Core owns redacted session semantics; host owns acquisition through WebView/platform stores |
| Local books | Core owns supported parser semantics and explicit unsupported-format policy |
| RSS | Core owns source import, parse, refresh state, read/starred state; host owns network fetch |
| Storage/cache/downloads | Core owns schemas, migrations, deterministic snapshots, progress, queue semantics |
| WebDAV/backup/sync | Core owns package/journal/conflict semantics; host owns actual WebDAV transport and credentials |
| Web API/export/import/admin | Decide Core contract versus product-gated/host-owned surfaces |

## Reader-Core Migration Classification

Every old Reader-Core asset must be classified before it is counted:

| Class | Meaning | Native action |
| --- | --- | --- |
| `migrate` | Behavior or tests should move into Native implementation | Rebuild cleanly in Rust tests/fixtures without changing old repo |
| `replay` | Old output is expected-result evidence, not implementation | Convert into corpus expected DTO/hash or conformance fixture |
| `host` | Adapter/UI/platform behavior remains outside Core | Define host contract and platform proof requirements |
| `archive` | Not part of Native parity target | Record why it is not migrated |

Priority old Core inputs:

- parser/rule DSL tests
- request DSL, URLSession/HTTP adapter behavior, headers/body/charset/retry
- cookie/session/login redacted behavior
- RECOVERY-29 JS/runtime behavior
- RECOVERY-30 cookie/session/login bridge
- RECOVERY-31 local book ingestion
- RECOVERY-32 local book library/runtime
- RECOVERY-33 unified remote/local reading runtime, cache, progress, downloads
- WebDAV, backup, sync, and export/import reports

## Target Architecture

```text
Legado source audit
  -> compatibility ledger

Old Reader-Core audit
  -> migration/replay/host/archive ledger

Reader-Core-Native
  -> protocol schemas and conformance fixtures
  -> Rust runtime, rule, JS, content, storage, local, RSS, sync crates
  -> C ABI: include/reader_core.h + reader-ffi
  -> platform SDK wrappers: Swift / JNI / NAPI

Host apps
  -> URLSession / OkHttp / Harmony HTTP
  -> WebView login/captcha/cookie capture
  -> file permissions, secure storage, background work, UI, packaging

Corpus benchmark
  -> CLI canonical result
  -> iOS canonical result
  -> Android canonical result
  -> HarmonyOS canonical result
  -> release gate decision
```

Core owns the deterministic product semantics. Hosts own platform capability
execution. The ABI should stay narrow: runtime lifecycle, command send, cancel,
destroy, status/error boundary, and callback-delivered events.

## Full Development Route

### Phase 0: Consolidation Baseline

Status: done for the Native repo.

Exit evidence:

- all observed Native worktrees/branches were merged into
  `codex/full-branch-directory-consolidation`
- no unmerged local branch remains outside the consolidated branch
- `cargo test --workspace` passed
- `cargo run -p reader-cli -- --conformance` passed
- `./scripts/ffi-smoke.sh` passed
- Android Java compile smoke and JNI C++ syntax smoke passed

### Phase 1: Legado Capability Ledger

Goal: define exactly what Native must be compatible with before more work is
called "parity".

Required outputs:

- `docs/compat/legado-capability-ledger.md`
- `docs/compat/legado-source-index.json`
- per-capability owner: Core, host, product-gated, policy-no-go, or out-of-scope
- per-capability evidence requirement: unit test, conformance fixture, corpus
  replay, host-app proof, manual approval, or policy closure

Exit condition:

- every implementation branch can point to a capability row, or explicitly mark
  itself as infrastructure.

### Phase 2: Old Reader-Core Migration Ledger

Goal: preserve useful old Core work instead of restarting from memory.

Required outputs:

- `docs/migration/reader-core-migration-ledger.md`
- `docs/migration/reader-core-test-port-plan.md`
- inventory of old tests, fixtures, adapters, reports, and RECOVERY artifacts
- each asset classified as migrate, replay, host, or archive

Exit condition:

- high-risk old Core behavior has a Native destination or an explicit archive
  reason.

### Phase 3: Native Contract And ABI Freeze

Goal: make the platform consumption boundary stable before platform agents
build too much wrapper code.

Scope:

- protocol versioning and JSON schemas
- runtime lifecycle, status, shutdown, cancel
- host operation registry and `host.request` / `host.complete` / `host.error`
- C ABI status codes, panic guard, last-error, borrowed callback buffer rules
- runtime config ingestion through ABI create path

Exit condition:

- CLI and C ABI can drive status, shutdown, cancel, host request/complete/error,
  structured errors, and runtime config without wrapper-specific behavior.

### Phase 4: Rule, JS, And Request Compatibility

Goal: close the high-risk Legado rule-chain gaps.

Scope:

- JSONPath filters, slices, unions, recursive descent, truthiness, missing values
- CSS selectors, `:contains`, `:containsOwn`, attributes, text/html extraction
- XPath predicates/namespaces and XML feed selectors
- regex extraction, replacement, capture groups, chained transforms
- JS helper behavior, host callback registry, timeout/cancel policy
- request DSL descriptors: method, headers, body, charset, redirect, retry,
  cookie/session policy

Exit condition:

- sanitized corpus can run `search -> detail -> toc -> content` using rule/JS
  paths without fake network or fake WebView success.

### Phase 5: Remote Reading Vertical

Goal: reproduce Legado webBook reading semantics end to end.

Required chain:

1. source import
2. search
3. detail
4. TOC
5. chapter content
6. pagination/windowing policy
7. cache/offline read
8. progress update and remap after TOC/content changes
9. duplicate chapter detection and canonical URL stability

Exit condition:

- the same source fixture yields the same canonical DTO/hash through CLI and at
  least one platform wrapper; later release gates require all three wrappers.

### Phase 6: Data, Local Book, RSS, And Sync

Goal: migrate old Reader-Core RECOVERY-31/32/33 and Legado non-webBook
capabilities.

Scope:

- TXT parser completion and EPUB/PDF/MOBI/UMD support policy
- local book library snapshots, duplicate/change detection, lazy chapter reads
- RSS import/parse/update/read/starred state
- storage schema, migrations, bookshelf queries, progress, cache, download queue
- WebDAV backup/restore, sync packages, journal, conflict policy

Exit condition:

- snapshot/import/export behavior is deterministic and covered by tests
- unsupported formats fail with explicit policy errors
- local/RSS/sync corpus cases can produce canonical results

### Phase 7: Platform SDK And Host Adapters

Goal: every host app loads the same Native Core commit and implements the host
capabilities required by the corpus.

Required platform proof:

- iOS: XCFramework, Swift SDK, URLSession host adapter, WebView/session adapter,
  App-side runtime lifecycle proof
- Android: JNI `.so`, Java/Kotlin SDK, OkHttp host adapter, WebView/session
  adapter, App-side runtime lifecycle proof
- HarmonyOS: NAPI `.so`, ArkTS SDK, HTTP/WebView/session adapters, HAP/device or
  platform-real runner proof

Exit condition:

- each host can run the benchmark driver for the same corpus case and emit the
  same canonical result hash.

### Phase 8: Corpus Benchmark And Release Gate

Goal: prove parity with repeatable, privacy-safe evidence.

Required outputs:

- approved/sanitized corpus with source manifests
- corpus runner for CLI, iOS, Android, HarmonyOS
- canonical DTO schema and hash rules
- per-capability pass/fail report
- release blocker register
- CI/nightly gate matrix

Exit condition:

- `coreParityComplete`, `legadoParityComplete`, and `productionReady` can be set
  only after benchmark evidence and platform proof exist.

### Phase 9: Old Core Retirement

Goal: remove duplicate Core dependencies only after Native has proof.

Scope:

- host apps switch their runtime paths to Native Core
- old Reader-Core assets are archived with migration mapping
- release documentation names Native Core as the production Core

Exit condition:

- no platform release path depends on old Reader-Core for covered capabilities.

## Independent Long-Term Goal Branches

Use `origin/codex/full-branch-directory-consolidation` as the base unless a
newer integration branch is explicitly selected. Each branch may read the whole
workspace, but writes should stay inside its owned paths.

### Branch 1: Legado Compatibility Ledger

Branch: `codex/goal-legado-compat-ledger`

Owned write paths:

- `docs/compat/**`
- `reports/compat/**`

Long-term objective:

- Produce the source-backed capability ledger that defines compatibility.

Restrictions:

- Read `/Users/minliny/Documents/legado`; do not edit it.
- Do not copy, translate, or rewrite GPL implementation code.
- Do not implement Native code in this branch.

Verification:

- every ledger row cites local Legado source paths
- every row has owner and evidence type
- no Native runtime claim is made from static source reading alone

Prompt:

```text
You are working in /Users/minliny/Documents/Reader-Core-Native on branch codex/goal-legado-compat-ledger.
Goal: build the source-backed Legado compatibility ledger for Reader-Core-Native.
Use /Users/minliny/Documents/legado as read-only input. Do not copy, translate, or rewrite GPL implementation code.
Create docs/compat/legado-capability-ledger.md and docs/compat/legado-source-index.json.
For each capability, record: Legado source path, behavior summary in your own words, owner(Core/host/product-gated/policy-no-go/out-of-scope), evidence required, priority, and dependent Native modules.
Do not modify crates, bindings, protocol, scripts, or host app repos.
Run markdown/link consistency checks you can run locally, then report the remaining unknowns.
```

### Branch 2: Reader-Core Migration Ledger

Branch: `codex/goal-reader-core-migration-ledger`

Owned write paths:

- `docs/migration/**`
- `reports/migration/**`

Long-term objective:

- Decide how existing Reader-Core behavior, tests, fixtures, and evidence move
  into Native.

Restrictions:

- Read `/Users/minliny/Documents/Reader-Core`; do not edit it.
- Treat the old repo's dirty state as an observed snapshot, not stable truth.
- Do not implement Native code in this branch.

Verification:

- every high-risk asset is classified as migrate, replay, host, or archive
- RECOVERY-29 through RECOVERY-33 are covered
- test-port plan identifies exact Native destination paths

Prompt:

```text
You are working in /Users/minliny/Documents/Reader-Core-Native on branch codex/goal-reader-core-migration-ledger.
Goal: inventory old Reader-Core behavior assets and produce a migration ledger for the Native rebuild.
Use /Users/minliny/Documents/Reader-Core as read-only input, including _archived_planning_2026-06-24.
Create docs/migration/reader-core-migration-ledger.md and docs/migration/reader-core-test-port-plan.md.
Classify every relevant test/report/fixture/adapter as migrate, replay, host, or archive. Cover parser/rule, request/network, JS/WebView, cookie/session, RECOVERY-31/32 local books, RECOVERY-33 unified runtime, RSS, WebDAV/sync, and platform adapters.
Do not edit the old Reader-Core repo and do not modify Native implementation files.
End with the highest-risk missing migration items and the first implementation branches they should feed.
```

### Branch 3: Rule, JS, And Request Parity

Branch: `codex/goal-rule-js-request-parity`

Owned write paths:

- `crates/reader-rule/**`
- `crates/reader-js/**`
- `crates/reader-content/**`
- `tests/fixtures/**`
- `fixtures/sanitized-corpus/**`
- focused docs under `reports/rule-js-request-parity/**`

Long-term objective:

- Close high-risk rule/JS/request compatibility gaps needed by the webBook
  reading chain.

Restrictions:

- Do not change C ABI signatures unless a separate ABI proposal is written.
- Do not fake WebView-only behavior; return host-required errors/contracts.
- Use Legado/Reader-Core ledgers when available; if unavailable, seed from
  `reports/legado-migration-master-audit/README.md`.

Verification:

- `cargo test -p reader-rule`
- `cargo test -p reader-js`
- `cargo test -p reader-content`
- corpus cases for selectors/request descriptors are added or updated

Prompt:

```text
You are working in /Users/minliny/Documents/Reader-Core-Native on branch codex/goal-rule-js-request-parity.
Goal: improve Legado-compatible rule, JS, and request descriptor behavior without changing the platform ABI.
Use docs/compat and docs/migration if present; otherwise seed from reports/legado-migration-master-audit/README.md.
Focus on JSONPath/CSS/XPath/Regex edge cases, rule chaining, QuickJS helper behavior, host callback policy, and request descriptors for method/headers/body/charset/redirect/retry/cookie.
Write tests in the relevant crates and add sanitized corpus fixtures when behavior needs cross-platform benchmark coverage.
Do not claim WebView/login parity unless the behavior is represented as a host-required contract.
Run cargo test -p reader-rule -p reader-js -p reader-content and record any remaining gaps.
```

### Branch 4: Remote Reading Corpus Runner

Branch: `codex/goal-remote-reading-corpus-runner`

Owned write paths:

- `tools/reader-cli/**`
- `crates/reader-runtime/**`
- `crates/reader-contract/**`
- `protocol/fixtures/**`
- `fixtures/sanitized-corpus/**`
- `reports/corpus-benchmark/**`

Long-term objective:

- Turn the remote reading vertical into a benchmarkable proof chain.

Restrictions:

- Do not change platform SDK wrapper APIs except through documented protocol
  additions.
- Do not use live private sources or persist raw credentials/cookies.
- Any protocol change must include schema and conformance fixture updates.

Verification:

- `cargo run -p reader-cli -- --conformance`
- `cargo test -p reader-runtime -p reader-contract -p reader-cli`
- corpus runner emits canonical DTO/hash for at least CLI cases

Prompt:

```text
You are working in /Users/minliny/Documents/Reader-Core-Native on branch codex/goal-remote-reading-corpus-runner.
Goal: build the corpus benchmark path for source import -> search -> detail -> toc -> chapter content -> progress.
Add or extend CLI runner support for sanitized corpus manifests and canonical DTO/hash output. Update protocol/conformance only when needed.
Keep raw source bodies, tokens, cookies, and credentials out of reports. Use fixtures/sanitized-corpus as the initial dataset.
Do not edit platform app repos in this branch.
Run cargo run -p reader-cli -- --conformance and focused runtime/CLI tests. Report which cases are CLI-only and what is still needed for iOS/Android/Harmony wrapper execution.
```

### Branch 5: Data, Local Book, RSS, And Sync Parity

Branch: `codex/goal-data-local-rss-sync-parity`

Owned write paths:

- `crates/reader-storage/**`
- `crates/reader-local-book/**`
- `crates/reader-rss/**`
- `crates/reader-sync/**`
- `fixtures/sanitized-corpus/**`
- `reports/data-local-rss-sync/**`

Long-term objective:

- Migrate old Core local/RSS/sync behavior and make deterministic data
  snapshots usable by all platforms.

Restrictions:

- Do not add platform-specific file picker, secure storage, or WebDAV network
  code to Core.
- Unsupported formats must fail explicitly; do not pretend EPUB/PDF/MOBI/UMD
  support exists before real parser policy is implemented.
- Keep transport credentials out of Core snapshots.

Verification:

- `cargo test -p reader-storage -p reader-local-book -p reader-rss -p reader-sync`
- snapshot import/export round trips are deterministic
- local/RSS/sync corpus fixtures have manifests and expected hashes where
  applicable

Prompt:

```text
You are working in /Users/minliny/Documents/Reader-Core-Native on branch codex/goal-data-local-rss-sync-parity.
Goal: close data, local-book, RSS, storage, WebDAV/sync, and backup/restore parity gaps using old Reader-Core migration evidence and Legado capability scope.
Focus on deterministic snapshots, schema validation, duplicate/change detection, lazy reads, RSS refresh state, sync package/journal merge rules, and explicit unsupported-format errors.
Do not implement host-owned network/file picker/secure-storage UI. Model those as host contracts when needed.
Run cargo test -p reader-storage -p reader-local-book -p reader-rss -p reader-sync and produce reports/data-local-rss-sync/status.md.
```

### Branch 6: Platform SDK And Host Adapter Proof

Branch: `codex/goal-platform-sdk-host-adapters`

Owned write paths:

- `bindings/ios/**`
- `bindings/android/**`
- `bindings/harmony/**`
- `scripts/build-ios-*`
- `scripts/build-android-*`
- `scripts/build-harmony-*`
- host app repos only when that task explicitly targets the host repo

Long-term objective:

- Prove iOS, Android, and HarmonyOS can load the same Native Core and satisfy
  host operations required by benchmark cases.

Restrictions:

- Platform wrappers consume ABI; they do not define Core semantics.
- No ABI signature change without a Core ABI branch and conformance update.
- Device/App claims require platform-real proof, not just wrapper compile.

Verification:

- iOS wrapper smoke and App-side adapter proof
- Android JNI build/smoke and App-side adapter proof
- Harmony NAPI build/smoke plus HAP/device or platform-real proof
- corpus runner output for each wrapper when available

Prompt:

```text
You are working in /Users/minliny/Documents/Reader-Core-Native on branch codex/goal-platform-sdk-host-adapters.
Goal: make iOS, Android, and HarmonyOS consume the same Native Core through the stable C ABI and produce platform-real host adapter evidence.
Do not change Core semantics in platform wrappers. If an ABI or protocol change is required, write the proposal and stop before broad edits.
Implement/verify wrapper lifecycle, event delivery, host.complete/host.error, and HTTP/WebView/session adapter contracts for corpus benchmark needs.
Core-side wrapper smoke is not App/device proof; label evidence precisely.
Run the available wrapper build/smoke commands for the local SDKs present and document missing SDK/toolchain blockers.
```

### Branch 7: Release Gates And Evidence Governance

Branch: `codex/goal-release-ci-evidence-governance`

Owned write paths:

- `docs/ci-gates/**`
- `evidence/release-readiness/**`
- `reports/release-gates/**`
- `.github/**` only if the task explicitly includes CI implementation

Long-term objective:

- Convert the compatibility/corpus/platform proof model into fail-closed gates
  and release decision evidence.

Restrictions:

- Do not overclaim host/device proof from Core smoke.
- CI jobs that require unavailable SDKs must fail closed or be scheduled only
  in the correct runner class.
- Privacy-sensitive corpus evidence must stay redacted.

Verification:

- gate matrix maps every release claim to a command or platform proof artifact
- release blocker register separates Core, host, corpus, policy, and tooling
  blockers
- CI changes, if made, are incremental and reversible

Prompt:

```text
You are working in /Users/minliny/Documents/Reader-Core-Native on branch codex/goal-release-ci-evidence-governance.
Goal: build the fail-closed release gate and evidence governance layer for Legado-compatible Native Core.
Map every release claim to required evidence: unit, conformance, corpus CLI, iOS wrapper, Android wrapper, Harmony wrapper, App/device proof, privacy approval, or policy no-go.
Do not claim App/device parity from Core-side smoke. Keep corpus reports privacy-safe.
Only edit .github workflows if explicitly requested; otherwise keep this as docs/evidence design.
Produce reports/release-gates/status.md and update docs/ci-gates as needed.
```

## Parallel Work Rules

- Branches can run in parallel if they obey owned write paths.
- A branch that needs a shared contract change should add a short proposal under
  its report directory instead of editing another branch's owned files.
- Implementation branches should consume the ledgers as they become available,
  but they do not have to wait for every other branch to finish.
- Merge completed branches through the smallest integration lane that can
  validate them.
- Before a merge, every branch must answer:
  1. Which Legado capability row does this close?
  2. Which old Reader-Core asset did it migrate, replay, host, or archive?
  3. Which Native protocol/C ABI contract did it change?
  4. Which platform wrappers must be updated?
  5. Which corpus benchmark case proves identical canonical results?

## Release Readiness Definition

The project is not release-ready until all of these are true:

- Legado capability ledger exists and high-priority rows have evidence status.
- Old Reader-Core migration ledger exists and high-risk assets are accounted for.
- Native protocol/C ABI is versioned, tested, and wrapper-consumable.
- Remote reading, local book, RSS, storage, and sync critical paths have
  deterministic tests and corpus cases.
- CLI, iOS, Android, and HarmonyOS can run the same approved corpus cases.
- Canonical result hashes match for critical paths.
- Host-owned gaps are explicitly tracked and not counted as Core parity.
- Privacy and source approval status is recorded for every real corpus case.

## Immediate Next Actions

1. Start `codex/goal-legado-compat-ledger`.
2. Start `codex/goal-reader-core-migration-ledger`.
3. Let implementation branches continue only if they can state their ledger seed
   and evidence target.
4. Build the corpus runner early enough that feature branches can attach proof
   instead of producing isolated smoke tests.
5. Treat platform wrapper work as consumption proof of the stable ABI, not as a
   place to define Core behavior.
