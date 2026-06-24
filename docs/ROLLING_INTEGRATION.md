# Rolling Integration

Reader-Core-Native uses rolling integration while multiple agents work in
parallel. Do not wait for every branch in a batch to finish. Integrate each
completed commit into the smallest lane that can validate it.

## Lanes

| Lane | Branch | Purpose | Waits for |
| --- | --- | --- | --- |
| Core foundation | `codex/core-foundation-integration` | Protocol, runtime, rule engine, QuickJS, ABI smoke | Only completed foundation branches |
| Core product | `codex/core-product-integration` | Remote reading vertical, content, storage, progress | Core foundation plus the completed product branch |
| Android | `codex/android-integration` | JNI bridge and Android host smoke | Core product baseline plus completed Android JNI branch |
| iOS | `codex/ios-integration` | Swift wrapper runtime smoke and iOS host proof | Core product baseline plus completed iOS wrapper/runtime branch |
| HarmonyOS | `codex/harmony-integration` | NAPI, ArkTS bridge, HAP package proof | Core product baseline and HarmonyOS whitelisted changes |

## Current Foundation Baseline

`codex/core-foundation-integration` currently includes:

- `codex/core-protocol-runtime`
- `codex/protocol-runtime-hardening`
- `codex/rule-engine-nonjs`
- `codex/rule-engine-edgecases`
- `codex/quickjs-runtime`
- `codex/quickjs-sandbox-hardening`

Validated gates:

- `cargo fmt --check`
- `./scripts/check-local.sh`
- `./scripts/build-local.sh`
- `./scripts/build-ohos.sh`
- `./scripts/build-harmony-napi.sh`

## Current Product Baseline

`codex/core-product-integration` currently includes the remote-reading vertical,
host HTTP contract, iOS Swift client smoke, and product status docs on top of
the foundation baseline. The product lane is complete for Core-side smoke:

- `remote.reading.v1` capability is declared in the JSON command schema.
- `http.execute` is declared as the host HTTP transport capability.
- Runtime commands cover source import, search, detail, toc, chapter content,
  and reading progress update.
- Remote commands support both fixture/inline responses and host
  request/complete response bodies.
- Content/storage/progress evidence is Core-side only; storage is V1 in-memory
  cache/progress, not a durable platform database.
- iOS Swift wrapper smoke compiles, links, and runs `core.info` /
  `runtime.ping` against the Core ABI, but it is not App/device integration.
- No platform App/device completion is claimed by this lane.

Recorded product-lane gates:

- `cargo test`
- `./scripts/check-local.sh`
- `./scripts/build-local.sh`
- `./scripts/build-ohos.sh`
- `./scripts/build-harmony-napi.sh`
- `./scripts/check-ios-swift-wrapper.sh`

## Command Pattern

Use `scripts/integration-queue.sh` when a branch is ready:

```bash
scripts/integration-queue.sh \
  codex/android-integration \
  origin/codex/core-product-integration \
  origin/codex/<android-jni-branch>
```

Optional gates:

```bash
RUN_OHOS=1 RUN_NAPI=1 PUSH=1 scripts/integration-queue.sh \
  codex/harmony-integration \
  origin/codex/core-product-integration \
  origin/codex/<harmony-app-integration-branch>
```

## Rules

- Never integrate from a dirty worktree.
- Use a separate worktree for each integration lane.
- Merge source branches in dependency order.
- Stop on the first conflict or failing gate; fix the integration branch, not
  the source branch, unless the source branch itself is wrong.
- Do not move the HarmonyOS lane based on HAP packaging alone. Device/runtime
  claims require platform-real evidence.
- Do not merge unreviewed HarmonyOS repository-wide dirty changes. Stage only
  task-owned NAPI/ArkTS/HAP files.
- Do not mark Android JNI complete from local placeholder branches. Require a
  clean pushed branch with JNI smoke evidence.
- Do not treat iOS Swift wrapper smoke as host adapter or App/device proof.
  URLSession/WebView/App integration needs separate evidence.

## Next Trigger

The product lane has already integrated the Core-side remote-reading vertical.
The next rolling triggers are platform integration branches:

- Android JNI: integrate a clean pushed JNI smoke branch into
  `codex/android-integration` on top of `origin/codex/core-product-integration`.
- iOS host transport: integrate only after URLSession/WebView/App-side evidence,
  not from Core-side wrapper smoke alone.
- HarmonyOS app-side integration: integrate NAPI/ArkTS/HAP changes only after
  whitelisted App-side evidence; do not claim device/runtime parity without
  platform-real proof.
