# Harmony NAPI 状态

## 范围

本文只跟踪 Harmony wrapper。当前工作范围限制在 `bindings/harmony/**`、
`scripts/build-harmony-napi.sh`、`scripts/build-ohos.sh`。

## 本线已闭环

- Runtime lifecycle：NAPI 可以创建和释放 Reader-Core runtime handle。
- Lifecycle smoke：NAPI 暴露 `lifecycleSmoke(iterations)`，可反复 create runtime、
  send `runtime.ping`、读取 result event、destroy runtime。
- Command/event path：NAPI 可以发送 JSON command，并从 thread-safe queue 读取已复制的
  Core event。
- Command guard：SDK 在 dispatch 到 native 前拒绝 empty command method、non-object
  command params、non-object `host.complete` result。
- Timeout guard：SDK 在 native polling 前拒绝负数/非整数 `timeoutMs` 和非正 `pollMs`。
- Cancellation：NAPI 暴露 `cancelRequest`，底层调用 `rc_runtime_cancel`。
- Host bus 最小回路：`host.request` 可读取并用 `host.complete` 回复；SDK helper 等待
  原 request result 时可自动完成 host request。
- 交错 event 处理：SDK 等待指定 request result 时会保留 unrelated events，
  避免其他 request 的 pending `host.request` 破坏当前 command/result flow。
- Event validation：SDK event parsing 会在进入 request waiting 或 host completion 逻辑前
  拒绝 malformed `result`、`error`、`host.request` payload。
- Host error path：NAPI 暴露 `failHostRequest`，通过 `host.error` JSON command path；
  host request handler 抛错时 SDK 会自动发送 `host.error`。
- SDK behavior smoke：`bindings/harmony/sdk/reader_core.test.ts` 使用 fake native module
  验证 `runtime.ping`、`host.complete`、handler failure -> `host.error`、command input
  rejection、timeout option rejection、unrelated event queuing、malformed native event
  rejection、`cancelRequest` dispatch。
- ArkTS package entry：`bindings/harmony/Index.ets` 导入 `libreader_core_napi.so` 并暴露
  `createReaderCoreRuntime` 与 `runHarmonyNapiSmoke`。
- Device smoke report 入口：`captureHarmonyNapiSmokeReport` 验证 HAP-side smoke result，
  返回可归档的 pass/fail structure；native loading 或 runtime execution 抛错时也返回
  structured failure report。`runHarmonyNapiSmokeReport` 保持同样检查，但失败时抛错。
- Device smoke artifact 入口：`captureHarmonyNapiSmokeArtifact` 包装稳定 artifact name、
  pass/fail summary 和 raw report payload；`runHarmonyNapiSmokeArtifact` 保持 gate-style
  failure semantics。
- Build evidence：OHOS 与 Harmony scripts 输出 deterministic artifact path、SHA-256、
  byte size、tool version、NAPI symbol evidence、package-ready Harmony directory manifest。

## 当前 SDK surface

- Native NAPI exports：`abiVersion`、`createRuntime`、`releaseRuntime`、`sendCommand`、
  `cancelRequest`、`readEvent`、`pendingEventCount`、`completeHostRequest`、
  `failHostRequest`、`pingSmoke`、`hostSmoke`、`lifecycleSmoke`。
- TypeScript/ArkTS wrapper：`bindings/harmony/sdk/reader_core.ts` 将 native exports 包成
  `ReaderCoreRuntime`，包括 `coreInfo`、`ping`、`hostSmoke`、generic `request`、
  explicit `readEvent`、explicit `completeHostRequest`、explicit `failHostRequest`。
- Smoke report helper：`bindings/harmony/sdk/smoke_report.ts` 校验 lifecycle、
  `core.info`、`runtime.ping`、`runtime.hostSmoke` 输出，并生成 deterministic JSON report，
  可用于 device-log archival。
- Package entry：`bindings/harmony/oh-package.json5` 指向 `Index.ets`。
- Package artifact：`scripts/build-harmony-napi.sh` 组装
  `target/harmony-napi/arm64-v8a/package`，包含 `.so`、ArkTS entry、非测试 SDK 文件和
  status/readme 文件，并生成 `harmony-package-manifest.sha256`。

## ABI 约束

- ABI v1 的 `rc_runtime_create`、`rc_runtime_send`、`rc_runtime_cancel` 只返回 integer
  status code。除非 Core/FFI 增加 last-error 或 direct out-buffer ABI，否则 Harmony 无法
  暴露 structured synchronous failure object。本 lane 不改 Core/FFI。
- ABI v1 event 是 callback-only borrowed buffer。Harmony 在 callback 返回前把 event
  bytes 复制到 NAPI-owned queue，再通过 `readEvent` 暴露 polling。
- `host.complete` 有意通过 `rc_runtime_send` 发送 JSON command；v1 没有单独的 host
  completion C ABI function。
- `host.error` 同样受 v1 约束，通过 `rc_runtime_send` 发送。

## 未完成 Harmony 工作

- 在签名 HAP 中于设备上运行 `captureHarmonyNapiSmokeArtifact`，并将 formatted artifact
  output 与本地 OHOS/Harmony build evidence 一起归档。
