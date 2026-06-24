# Cleanup Inventory

Date: 2026-06-25

This is a cleanup audit, not a destructive cleanup. No worktree, directory, or
branch was removed while producing this inventory.

## Direct Answer

The workspace is not cleaned up.

What has been done:

- The Legado migration audit was consolidated under
  `reports/legado-migration-master-audit`.
- Relevant Documents directories and Native worktrees were indexed.
- The audit branch was committed as `e2317d8`.

What has not been done:

- No Native worktree was removed.
- No Documents directory was deleted.
- No local branch was deleted.
- No dirty repository was modified.
- No completed branch was merged into the integration base.

## Documents Directory Inventory

| Path | Size | Git state | Cleanup decision |
| --- | ---: | --- | --- |
| `/Users/minliny/Documents/Reader for MacOS` | 4.0K | non-git | Small placeholder. Can be removed or archived only after user confirmation. |
| `/Users/minliny/Documents/Reader for Android_design_docs` | 48K | non-git | Design-doc folder. Keep unless confirmed obsolete. |
| `/Users/minliny/Documents/Reader for Windows` | 100K | git, status entries observed | Do not delete until repo state is inspected; HEAD display was incomplete in the quick audit. |
| `/Users/minliny/Documents/Reader-Core-Native-legado-master-audit` | 4.3M | clean, `codex/legado-migration-master-audit` | Audit output worktree. Can be removed after its commit is pushed or merged. |
| `/Users/minliny/Documents/Reader for HarmonyOS` | 44M | dirty, `codex/harmony-napi-runtime`, 57 status entries | Do not touch from Native cleanup. Host-app work in progress. |
| `/Users/minliny/Documents/legado` | 101M | clean, `master`, `da17bb2be` | Keep read-only as compatibility baseline. |
| `/Users/minliny/Documents/Reader-Core-Native-harmony-napi-integration` | 344M | clean, `codex/harmony-napi-integration` | Keep until Harmony NAPI branch is merged or explicitly archived. |
| `/Users/minliny/Documents/Reader-Core-Native-rule-js-compat-clean` | 522M | dirty, `codex/reader-rule-js-compat-clean`, 2 status entries | Do not remove. Needs commit/test/audit first. |
| `/Users/minliny/Documents/Reader-Core-Native-data-subsystem-storage` | 670M | clean, `codex/data-subsystem-storage-cache-coverage` | Keep until data subsystem branch is merged or replayed. |
| `/Users/minliny/Documents/Reader UI` | 677M | clean, `main` | Separate UI repo. Not Native cleanup scope. |
| `/Users/minliny/Documents/Reader-Core-Native-c-abi-worktree` | 946M | clean, `codex/reader-core-c-abi-stable-boundary` | Keep until C ABI branch is merged or replayed. |
| `/Users/minliny/Documents/Reader for iOS` | 1.3G | dirty, `main`, 53 status entries | Do not touch from Native cleanup. Host-app work in progress. |
| `/Users/minliny/Documents/Reader for Android` | 2.1G | clean, `main` | Keep as Android host-app integration target. |
| `/Users/minliny/Documents/Reader-Core-Native` | 3.9G | clean, `codex/reader-core-runtime-protocol` | Active Native worktree. Keep. |
| `/Users/minliny/Documents/Reader-Core` | 13G | dirty, `main`, 815 status entries | Do not touch. Old Core migration/evidence source with many dirty/archived files. |

## Registered Native Worktrees

| Worktree | Branch | Head | Status | Cleanup decision |
| --- | --- | --- | --- | --- |
| `/Users/minliny/Documents/Reader-Core-Native` | `codex/reader-core-runtime-protocol` | `fc626d7` | clean | Keep. Active runtime/protocol worktree. |
| `/private/tmp/ci-gate-design-wt` | `codex/goal-ci-gate-design` | `d5d1b08` | clean | Can remove after docs are merged/replayed. |
| `/private/tmp/goal-host-app-contracts-wt` | `codex/goal-host-app-contracts` | `ee44979` | clean | Can remove after docs are merged/replayed. |
| `/private/tmp/release-evidence-wt` | `codex/goal-release-evidence` | `f3e3d4d` | clean | Can remove after evidence is merged/replayed. |
| `/Users/minliny/Documents/Reader-Core-Native-c-abi-worktree` | `codex/reader-core-c-abi-stable-boundary` | `45aaec4` | clean | Keep until ABI integration is complete. |
| `/Users/minliny/Documents/Reader-Core-Native-data-subsystem-storage` | `codex/data-subsystem-storage-cache-coverage` | `3628055` | clean | Keep until data integration is complete. |
| `/Users/minliny/Documents/Reader-Core-Native-harmony-napi-integration` | `codex/harmony-napi-integration` | `4b9f9aa` | clean | Keep until Harmony integration is complete. |
| `/Users/minliny/Documents/Reader-Core-Native-legado-master-audit` | `codex/legado-migration-master-audit` | `e2317d8` | clean | Can remove after audit commit is pushed or merged. |
| `/Users/minliny/Documents/Reader-Core-Native-rule-js-compat-clean` | `codex/reader-rule-js-compat-clean` | `66e8151` | dirty | Do not remove. |
| `/Users/minliny/Documents/Reader-Core-Native/.claude/worktrees/android-jni-sdk` | `codex/android-jni-sdk` | `8eb7b99` | clean | Keep until Android JNI integration is complete. |
| `/Users/minliny/Documents/Reader-Core-Native/.wt-goal-sanitized-corpus` | `codex/goal-sanitized-corpus` | `87b8983` | clean | Can remove after corpus branch is merged/replayed. |

## Local Branch State

Observed local Native branches: 45.

Branches already merged into `origin/codex/core-product-integration`:

- `codex/ci-ios-wrapper-smoke`
- `codex/cli-host-http-smoke`
- `codex/core-foundation-integration`
- `codex/core-product-integration`
- `codex/core-protocol-runtime`
- `codex/cxx-abi-smoke`
- `codex/docs-product-status`
- `codex/ffi-negative-smoke`
- `codex/http-host-contract`
- `codex/ios-modulemap-ci`
- `codex/ios-swift-client-smoke`
- `codex/ios-swift-wrapper-smoke`
- `codex/ios-xcframework-smoke`
- `codex/protocol-runtime-hardening`
- `codex/quickjs-runtime`
- `codex/quickjs-sandbox-hardening`
- `codex/remote-reading-vertical`
- `codex/rule-engine-edgecases`
- `codex/rule-engine-nonjs`
- `main`

Do not delete `main` or the active integration branch. The other merged feature
branches are branch-delete candidates after confirming no open work depends on
their names.

Unmerged local branches with registered worktrees:

- `codex/android-jni-sdk`
- `codex/data-subsystem-storage-cache-coverage`
- `codex/goal-ci-gate-design`
- `codex/goal-host-app-contracts`
- `codex/goal-release-evidence`
- `codex/goal-sanitized-corpus`
- `codex/harmony-napi-integration`
- `codex/legado-migration-master-audit`
- `codex/reader-core-c-abi-stable-boundary`
- `codex/reader-core-runtime-protocol`
- `codex/reader-rule-js-compat-clean`

These cannot be deleted until their worktrees are removed and their commits are
merged, pushed, or intentionally abandoned.

Unmerged local branches without worktrees:

- `codex/android-integration`
- `codex/android-jni-sdk-20260625`
- `codex/android-jni-smoke`
- `codex/android-wrapper-integration`
- `codex/c-abi-runtime`
- `codex/data-subsystem-content-library`
- `codex/data-subsystem-local-book-library`
- `codex/data-subsystem-next`
- `codex/data-subsystem-rss-snapshots`
- `codex/data-subsystem-storage-shelf-query`
- `codex/data-subsystem-storage-snapshots`
- `codex/data-subsystem-sync-journal`
- `codex/data-subsystem-sync-packages`
- `codex/data-subsystem`
- `codex/ios-swift-sdk`
- `codex/ios-wrapper-integration`
- `codex/local-content-runtime`
- `codex/product-docs-rollup`
- `codex/reader-core-c-abi-boundary`
- `codex/reader-rule-js-compat`
- `codex/rule-engine-parity`
- `codex/storage-runtime`

These need per-branch diff audit before deletion. Several look like old
intermediate lanes, but they are not safe to delete just from naming.

## Safe Cleanup Sequence

1. Push or merge the audit branch, then remove only the audit worktree:
   `git worktree remove /Users/minliny/Documents/Reader-Core-Native-legado-master-audit`
2. Merge/replay Class A branches, then remove their worktrees:
   `goal-ci-gate-design`, `goal-host-app-contracts`, `goal-release-evidence`,
   `goal-sanitized-corpus`.
3. Run a per-branch diff audit for unmerged branches without worktrees.
4. Delete only branches proven merged or intentionally abandoned with
   `git branch -d <branch>`.
5. Avoid `git branch -D`, `rm -rf`, and host-app cleanup until the exact target
   branch/content is confirmed.

## Immediate Cleanup Candidates

These are the only low-risk cleanup candidates, and even these should wait for
confirmation because they remove local worktree directories:

- `/Users/minliny/Documents/Reader-Core-Native-legado-master-audit`
- `/private/tmp/ci-gate-design-wt`
- `/private/tmp/goal-host-app-contracts-wt`
- `/private/tmp/release-evidence-wt`
- `/Users/minliny/Documents/Reader-Core-Native/.wt-goal-sanitized-corpus`

Everything else either contains active work, dirty files, host-app state,
Legado baseline, or unmerged Native implementation work.
