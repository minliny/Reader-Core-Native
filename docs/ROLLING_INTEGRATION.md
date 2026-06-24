# Rolling Integration

Reader-Core-Native uses rolling integration while multiple agents work in
parallel. Do not wait for every branch in a batch to finish. Integrate each
completed commit into the smallest lane that can validate it.

## Lanes

| Lane | Branch | Purpose | Waits for |
| --- | --- | --- | --- |
| Core foundation | `codex/core-foundation-integration` | Protocol, runtime, rule engine, QuickJS, ABI smoke | Only completed foundation branches |
| Core product | `codex/core-product-integration` | Remote reading vertical, content, storage, progress | Core foundation plus the completed product branch |
| HarmonyOS | `codex/harmony-integration` | NAPI, ArkTS bridge, HAP package proof | Core ABI/protocol baseline and HarmonyOS whitelisted changes |

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

## Command Pattern

Use `scripts/integration-queue.sh` when a branch is ready:

```bash
scripts/integration-queue.sh \
  codex/core-product-integration \
  origin/codex/core-foundation-integration \
  origin/codex/remote-reading-vertical
```

Optional gates:

```bash
RUN_OHOS=1 RUN_NAPI=1 PUSH=1 scripts/integration-queue.sh \
  codex/core-product-integration \
  origin/codex/core-foundation-integration \
  origin/codex/remote-reading-vertical
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

## Next Trigger

When `origin/codex/remote-reading-vertical` appears as a clean pushed branch,
immediately integrate it into `codex/core-product-integration` on top of
`origin/codex/core-foundation-integration`. This does not need to wait for
HarmonyOS.
