# 阻塞项

> 当前最高优先级文档是 `docs/LOCAL_REPO_MIGRATION_DIRECTIVE.md`。本文记录历史阻塞项；
> 后续 blocker 必须按本地仓库实际代码重新审计。

> 基线 `fb4c3a7`，round-01。阻塞项 = 阻止某级 release readiness 声明的事项。

## Core-side smoke release（V1 Core）

**无阻塞项。** round-01 全部 Core-side gate 绿（fmt / test / conformance / vertical /
ffi-smoke / iOS xcframework+wrapper / OHOS+NAPI .so）。

## App/device release

以下阻塞 App/device 发布声明（均未验证，非本证据包可解决，属 platform adapter / 平台工程范围）：

| # | 平台 | 阻塞项 | 影响 |
|---|------|--------|------|
| B1 | iOS | 未构建 iOS App 工程、未模拟器/真机启动 | 无法声明 iOS App 可发布 |
| B2 | iOS | host adapter 未接入（WebView 登录/Cookie、Keychain、文件沙箱、后台任务、签名分发） | 同上 |
| B3 | HarmonyOS | 未打 HAP、未 DevEco/真机启动、`Index.ets` + SDK TS 侧未跑 | 无法声明 HarmonyOS App 可发布 |
| B4 | Android | 无 Core-side 产物构建路径（`bindings/android` 仅 `.gitkeep`，无构建脚本，无 JNI 桥） | 无法声明 Android Core-side 产物可用，进而无法声明 Android App |
| B5 | 全平台 | 真实网络/TLS/HTTP Transport 属 platform adapter，Core-side smoke 不覆盖 | 真实书源联网验证缺失 |

## 非阻塞但需跟踪

- host FFI / iOS swift wrapper 链接时 `ld` 告警“object built for newer macOS version (26.5)
  than linked (26.0)”。非致命，但若后续要发布带 dSYM/严格版本对齐的产物需处理。
- `scripts/build-local.sh` 本轮未作为整脚本跑（各分量已分别覆盖）。如需“整脚本一键绿”证据，
  可在后续轮次补跑 `./scripts/build-local.sh` 端到端。
