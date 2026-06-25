# Reader Rust Core 架构

最高优先级入口：`docs/LOCAL_REPO_MIGRATION_DIRECTIVE.md`

本文描述当前 Rust 目标仓库 `Reader-Core-Native` 的架构。后续必须先扫描本地工作区，
确认目标仓库路径、Git 状态和分支后再开始修改。

## 架构目标

Rust Reader-Core 是 Reader 的唯一业务内核。iOS、Android、HarmonyOS 通过 C ABI 和
各自 wrapper 消费同一个 Core，不再各自维护长期分叉的业务实现。

```text
旧 Reader-Core
  -> 行为、测试、fixture、迁移任务

Rust Reader-Core
  -> C ABI
  -> Swift wrapper
  -> JNI/Kotlin wrapper
  -> Node-API/ArkTS wrapper

iOS / Android / HarmonyOS
  -> 平台 adapter
  -> UI / 权限 / WebView / 系统服务 / 打包
```

## 当前 Rust Core 结构

| 层 | 路径 | 职责 |
| --- | --- | --- |
| C ABI | `include/reader_core.h`、`crates/reader-ffi` | runtime lifecycle、send、cancel、destroy、status/error |
| Protocol | `crates/reader-contract`、`protocol/*.schema.json` | JSON command/event DTO、runtime config、host operation |
| Runtime | `crates/reader-runtime` | worker、request dispatch、host bus、remote reading flow |
| Rule | `crates/reader-rule` | CSS/XPath/JSONPath/Regex、链式规则、fallback |
| JS | `crates/reader-js` | QuickJS sandbox、timeout、callback、JSON conversion |
| Content | `crates/reader-content` | search/detail/toc/content extraction、normalization |
| Storage | `crates/reader-storage` | source/book/chapter/progress/cache/snapshot |
| Local book | `crates/reader-local-book` | TXT、本地书 library、章节 |
| RSS | `crates/reader-rss` | feed parsing、subscription state |
| Sync | `crates/reader-sync` | WebDAV/sync package、journal、conflict model |
| CLI | `tools/reader-cli` | 本地 conformance、fixture、benchmark 入口 |

## C ABI 边界

ABI v1 暴露：

- `rc_abi_version`
- `rc_runtime_create`
- `rc_runtime_send`
- `rc_runtime_cancel`
- `rc_runtime_destroy`

平台 wrapper 不直接调用 Rust 内部 crate。所有平台消费都必须通过 C ABI 和 JSON
command/event protocol。若平台需要新的业务能力，先在 Rust Core 和 protocol 中定义，
再更新 wrapper。

## Host operation 边界

Core 发出 `host.request`，平台用 `host.complete` 或 `host.error` 回复。

Core 负责：

- request descriptor
- correlation id
- retry/redirect/cookie/charset 语义
- structured error
- deterministic result model

平台负责：

- URLSession / OkHttp / Harmony HTTP
- WebView 登录、captcha、Cookie、DOM
- Keychain / Keystore / credential store
- 文件选择、目录权限、安全沙箱
- TTS、通知、后台任务
- UI 与 App lifecycle

## 当前已具备基础

- Rust workspace、C ABI、C/C++ smoke。
- JSON protocol 与 conformance fixture。
- remote reading V1 纵切。
- rule/JS/content 基础能力。
- local TXT、RSS、storage、sync 基础状态机。
- iOS Swift wrapper。
- Android JNI wrapper。
- HarmonyOS Node-API/ArkTS wrapper。

## 当前不可过度声明

- wrapper compile/link smoke 不等于 App/device 完成。
- Core-side fixture 不等于三端迁移完成。
- 单端验证不等于三端一致。
- 历史审计报告不等于当前本地代码事实。
- 旧 Reader-Core、iOS、Android、HarmonyOS 的实际代码必须重新审计并映射到 Rust。

## 基础验证命令

```bash
cargo fmt --check
cargo test --workspace
cargo run -p reader-cli -- --conformance
cargo run -p reader-cli -- --fixture-vertical tests/fixtures/remote_source/basic_source.json
./scripts/ffi-smoke.sh
./scripts/build-local.sh
./scripts/check-ios-swift-wrapper.sh
./scripts/build-android-jni.sh
./scripts/build-harmony-napi.sh
```

平台 SDK 不齐时，对应命令必须 fail-closed 或明确记录 blocker。
