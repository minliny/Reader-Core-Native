# Branch Integration Ledger

Date: 2026-06-25

This ledger turns the current completed branch inventory into a merge strategy.
It does not merge branches directly because several branches share earlier
cross-agent history. The right next step is path-level integration or selective
replay, not blind merge.

## Merge Classes

### Class A: independent, low-risk consolidation

These branches are documentation or corpus-data lanes and can be merged or
replayed independently once the target base is selected:

| Branch | Worktree | Scope | Required validation |
| --- | --- | --- | --- |
| `codex/goal-ci-gate-design` | `/private/tmp/ci-gate-design-wt` | CI gate design under `docs/ci-gates/**` | Markdown review, no runtime validation required. |
| `codex/goal-host-app-contracts` | `/private/tmp/goal-host-app-contracts-wt` | host/Core responsibility contracts under `docs/host-app-contracts/**` | Markdown review plus consistency with C ABI events. |
| `codex/goal-release-evidence` | `/private/tmp/release-evidence-wt` | release readiness evidence under `evidence/release-readiness/**` | Check that it does not claim App/device parity. |
| `codex/goal-sanitized-corpus` | `/Users/minliny/Documents/Reader-Core-Native/.wt-goal-sanitized-corpus` | sanitized seed fixtures and audit report | Privacy grep, manifest schema check, corpus runner stub once available. |

### Class B: core foundation, merge with path-level audit

These branches are high value but overlap with previous agent history:

| Branch | Worktree | Scope | Required validation |
| --- | --- | --- | --- |
| `codex/reader-core-runtime-protocol` | `/Users/minliny/Documents/Reader-Core-Native` | runtime status/shutdown, cancel, host completions, protocol conformance | `cargo test -p reader-contract -p reader-runtime`; inspect platform files before merge. |
| `codex/reader-core-c-abi-stable-boundary` | `/Users/minliny/Documents/Reader-Core-Native-c-abi-worktree` | `include/reader_core.h`, `crates/reader-ffi`, iOS module map, FFI smoke | `cargo test -p reader-ffi`; `./scripts/ffi-smoke.sh`; ABI header review. |
| `codex/data-subsystem-storage-cache-coverage` | `/Users/minliny/Documents/Reader-Core-Native-data-subsystem-storage` | content/local-book/RSS/storage/sync crates and cache coverage planning | `cargo test -p reader-content -p reader-local-book -p reader-rss -p reader-storage -p reader-sync`; compare ABI/protocol carry-over before merge. |

### Class C: platform lanes, integrate after core ABI shape freezes

These should not set Core semantics. They consume the stable Core/ABI shape:

| Branch | Worktree | Scope | Required validation |
| --- | --- | --- | --- |
| `codex/android-jni-sdk` | `/Users/minliny/Documents/Reader-Core-Native/.claude/worktrees/android-jni-sdk` | JNI bridge, CMake, Kotlin sample, command/event bridge | NDK build, JNI smoke, Android app adapter smoke. |
| `codex/harmony-napi-integration` | `/Users/minliny/Documents/Reader-Core-Native-harmony-napi-integration` | NAPI wrapper, ArkTS SDK helpers, Harmony smoke artifacts | OHOS build script, NAPI smoke, HAP/device proof before release claims. |

### Class D: active, do not merge yet

| Branch | Worktree | Scope | Blocker |
| --- | --- | --- | --- |
| `codex/reader-rule-js-compat-clean` | `/Users/minliny/Documents/Reader-Core-Native-rule-js-compat-clean` | rule and JS compatibility work | Dirty files: `crates/reader-rule/src/lib.rs`, `crates/reader-rule/tests/rule_parity.rs`; needs commit and focused tests first. |

## Recommended Integration Order

1. Consolidate Class A first so the project has source-truth, host-contract, CI,
   release, and corpus scaffolding in one base.
2. Integrate runtime/protocol core-only work from
   `codex/reader-core-runtime-protocol`.
3. Integrate C ABI stable boundary from
   `codex/reader-core-c-abi-stable-boundary`.
4. Integrate Android and Harmony platform lanes against the frozen ABI.
5. Integrate data subsystem once protocol/ABI conflicts are resolved.
6. Finish and validate rule/JS branch, then integrate it as the Legado
   compatibility execution lane.

## Required Merge Review Checks

Every integration PR should answer:

1. Which Legado compatibility capability does this close?
2. Which existing Reader-Core asset did it migrate, replay, host, or archive?
3. Which Native/C ABI contract did it change?
4. Which platform wrappers must be updated?
5. Which corpus benchmark case proves identical canonical results?

If the answer to item 5 is "none", the branch can be merged as infrastructure,
but it cannot be counted as Legado parity closure.
