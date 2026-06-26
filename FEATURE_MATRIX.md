# 能力状态矩阵

最高优先级入口：`docs/LOCAL_REPO_MIGRATION_DIRECTIVE.md`

状态定义：

- 已完成：当前 Rust 目标仓库有实现路径和验证命令。
- 部分完成：已有基础，但不能声明迁移完成。
- Gap：尚未实现，或缺少本地旧 Core/三端验证证据。
- 平台负责：不进入 Rust 业务内核，由 iOS/Android/HarmonyOS adapter 实现。

## Core / ABI / Protocol

| 能力 | Owner | 状态 | 证据 |
| --- | --- | --- | --- |
| C ABI lifecycle | Rust Core | 已完成 | `include/reader_core.h`、`crates/reader-ffi`、`./scripts/ffi-smoke.sh` |
| JSON command/event protocol | Rust Core | 已完成 | `crates/reader-contract`、`protocol/*.schema.json` |
| Runtime worker / request dispatch | Rust Core | 已完成 | `crates/reader-runtime` |
| Host operation bus | Rust Core + 平台 adapter | 已完成路由 | `host.request`、`host.complete`、`host.error` |
| Runtime config | Rust Core | 部分完成 | `protocol/reader-runtime-config.schema.json` |
| Structured error / last error | Rust Core | 部分完成 | C ABI 已有基础，wrapper 暴露需继续补齐 |

## 规则、JS、请求描述

| 能力 | Owner | 状态 | 证据 |
| --- | --- | --- | --- |
| Regex | Rust Core | 已完成 | `crates/reader-rule` Legado DSL dispatch via `execute_legado_rule` |
| JSONPath | Rust Core | 已完成 | `crates/reader-rule` Legado DSL dispatch via `execute_legado_rule` |
| CSS selector | Rust Core | 部分完成 | `crates/reader-rule` |
| XPath | Rust Core | 已完成 | `crates/reader-rule` Legado DSL dispatch via `execute_legado_rule` |
| 链式规则 / fallback | Rust Core | 部分完成 | `crates/reader-rule` |
| QuickJS sandbox | Rust Core | 部分完成 | `crates/reader-js` |
| JS host callback | Rust Core + 平台 adapter | 部分完成 | sandbox 有基础，需接通平台能力 |
| HTTP request descriptor | Rust Core | 部分完成 | `http.execute` contract |
| Cookie / Session / Redirect | Rust Core + 平台 adapter | Gap | 需从旧 `Reader-Core` 代码迁移和验证 |

## 阅读链路

| 能力 | Owner | 状态 | 证据 |
| --- | --- | --- | --- |
| Source import | Rust Core | 部分完成 | remote reading V1 |
| Search | Rust Core | 部分完成 | fixture vertical |
| Detail | Rust Core | 部分完成 | fixture vertical |
| TOC | Rust Core | 部分完成 | fixture vertical |
| Chapter content | Rust Core | 部分完成 | fixture vertical |
| Pagination / continuation | Rust Core | Gap | 需旧 Core 迁移和 corpus 证明 |
| Progress update/remap | Rust Core | 部分完成 | storage 基础能力 |
| Offline cache | Rust Core | Gap | 需 storage/cache 迁移 |

## 本地书、RSS、同步、数据

| 能力 | Owner | 状态 | 证据 |
| --- | --- | --- | --- |
| TXT | Rust Core | 部分完成 | `crates/reader-local-book` |
| EPUB | Rust Core | Gap | 需从旧 Core/平台现状审计 |
| SQLite schema/migration | Rust Core | Gap | 需落地持久化模型 |
| Cache/progress/history/download queue | Rust Core | 部分完成 | `crates/reader-storage` 有基础 |
| RSS | Rust Core | 部分完成 | `crates/reader-rss` |
| WebDAV/sync/diff/recovery | Rust Core + 平台 adapter | 部分完成 | `crates/reader-sync` 有模型，平台 transport 待接入 |
| TTS 数据契约 | Rust Core + 平台 adapter | Gap | Core 定义契约，平台执行播放 |

## 平台 wrapper

| 平台 | 状态 | 说明 |
| --- | --- | --- |
| iOS Swift wrapper | 部分完成 | 能消费 C ABI smoke；App-side URLSession/WKWebView/Keychain/File/TTS 仍需迁移验证 |
| Android JNI/Kotlin wrapper | 部分完成 | JNI wrapper 已有；App-side OkHttp/WebView/Keystore/SAF/TTS 仍需迁移验证 |
| HarmonyOS Node-API/ArkTS wrapper | 部分完成 | NAPI wrapper 已有；HAP/device 和平台 adapter 仍需验证 |

## 平台负责能力

以下能力不进入 Rust 业务内核实现，但必须有 host contract：

- 真实 HTTP/TLS/socket 执行。
- WebView 登录、captcha、Cookie、DOM。
- Keychain / Keystore / credential store。
- 文件选择、目录授权、系统沙箱。
- TTS 播放和系统媒体能力。
- UI、导航、主题、通知、后台任务。
- App packaging、signing、distribution。

## 迁移完成判定

能力不能仅凭 Rust 单测或 wrapper smoke 标记为迁移完成。完成必须同时满足：

1. 已审计旧 `Reader-Core` 或对应平台代码。
2. Rust Core 有实现和测试。
3. 至少一个 CLI fixture 可运行。
4. 三端 wrapper/host adapter 有对应验证计划。
5. release 前三端同一 fixture/corpus canonical result 一致。
