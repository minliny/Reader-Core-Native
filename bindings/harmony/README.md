# Reader-Core Harmony Binding

> 当前最高优先级文档是 `docs/LOCAL_REPO_MIGRATION_DIRECTIVE.md`。本文只说明 HarmonyOS
> Node-API wrapper；HarmonyOS App 迁移必须以本地 `Reader for HarmonyOS` 代码为事实来源。

该 package 是 `libreader_core_napi.so` 的 Harmony 侧 wrapper。

## 文件

- `native/reader_napi.cpp`：NAPI bridge，负责 runtime handle、event queue copy、
  command send、cancellation、host request completion/failure。
- `sdk/reader_core.ts`：native exports 的 typed SDK wrapper。
- `sdk/smoke_report.ts`：验证 smoke output，并格式化 device-log report。
- `sdk/reader_core.test.ts`：使用 fake-native 的 SDK smoke tests，可用 Bun 运行。
- `Index.ets`：ArkTS 入口，导入 `libreader_core_napi.so`，并暴露
  `createReaderCoreRuntime`、`runHarmonyNapiSmoke`、
  `captureHarmonyNapiSmokeReport`、`runHarmonyNapiSmokeReport` 以及 smoke artifact
  helper。
- `STATUS.md`：当前集成状态和 ABI 约束。

## 构建输出

`scripts/build-harmony-napi.sh` 生成可打包目录：

```text
target/harmony-napi/arm64-v8a/package
```

该目录包含 `oh-package.json5`、`Index.ets`、非测试 SDK `.ts` 文件、`README.md`、
`STATUS.md`、`libs/arm64-v8a/libreader_core_napi.so`。同一次构建还会写入
`target/harmony-napi/arm64-v8a/harmony-package-manifest.sha256`，记录每个 package 文件
的 deterministic SHA-256 和 byte size。

## Device smoke 入口

将 `libreader_core_napi.so` 打进 Harmony App 后调用：

```ts
import {
  captureHarmonyNapiSmokeArtifact,
  formatHarmonyNapiSmokeArtifact,
} from '@reader/core-harmony';

const artifact = await captureHarmonyNapiSmokeArtifact();
const output = formatHarmonyNapiSmokeArtifact(artifact);
```

Smoke 会创建 runtime，运行 native `lifecycleSmoke`，调用 `core.info` 与
`runtime.ping`，通过 `host.request` / `host.complete` 跑 `runtime.hostSmoke`，验证结果
shape，然后释放 runtime。

`captureHarmonyNapiSmokeReport` 在 native loading 或 runtime execution 抛错时返回结构化
failure report，方便 device log 仍可归档 JSON 结果。
`captureHarmonyNapiSmokeArtifact` 在 report 外包装稳定 artifact name、status、pass/fail
count，便于 device-log archival。需要失败即抛错的 gate caller 可使用
`runHarmonyNapiSmokeReport` 或 `runHarmonyNapiSmokeArtifact`。

在签名 HAP 于设备上运行后，应将 formatted artifact output 与本地 build evidence 一起归档。
