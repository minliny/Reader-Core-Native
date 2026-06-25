# Reader-Core-Native 能力矩阵

状态定义：

- 已完成：当前分支有实现路径和验证命令。
- 部分完成：已有可用基础，但不能声明产品能力完成。
- Gap：当前未实现或缺少关键证据。
- Host/app：按架构边界由平台宿主负责。

## Core protocol 与 runtime

| 能力 | Owner | 状态 | 证据 |
| --- | --- | --- | --- |
| ABI v1 lifecycle | Core | 已完成 | `include/reader_core.h`、`crates/reader-ffi/src/lib.rs`、`./scripts/ffi-smoke.sh` |
| JSON protocol v1 command/event DTO | Core | 已完成 | `crates/reader-contract/src/*.rs`、`protocol/*.schema.json` |
| `core.info` capability advertisement | Core | 已完成 | `crates/reader-contract/src/core_info.rs` |
| Runtime worker 与 request dispatch | Core | 已完成 | `crates/reader-runtime/src/runtime.rs` |
| `runtime.status` / `runtime.shutdown` | Core | 已完成 | conformance fixture 与 runtime 测试 |
| 取消 pending request | Core | 已完成 | `runtime.cancel` conformance |
| Host bus `host.request` / `host.complete` / `host.error` | Core protocol + host execution | routing 已完成 | `protocol/fixtures/conformance/host/*.json` |
| Runtime config schema | Core | 部分完成 | `protocol/reader-runtime-config.schema.json`、`crates/reader-contract/src/config.rs` |

## Remote reading V1

| 能力 | Owner | 状态 | 证据 |
| --- | --- | --- | --- |
| `remote.reading.v1` capability | Core | 已完成 | `crates/reader-contract/src/lib.rs` |
| `source.import` | Core | 已完成 | `crates/reader-runtime/src/remote.rs` |
| `book.search` with prefetched response | Core | 已完成 | `tools/reader-cli/tests/fixture_vertical.rs` |
| `book.search` via `http.execute` | Core contract + host transport | contract 已完成 | `crates/reader-runtime/src/remote.rs` |
| `book.detail` | Core | V1 已完成 | `crates/reader-runtime/src/remote.rs` |
| `book.toc` | Core | V1 已完成 | `crates/reader-runtime/src/remote.rs` |
| `chapter.content` rule path | Core | V1 已完成 | `crates/reader-content/src/lib.rs` |
| JS network path | Core + host | 部分完成 | sandbox 有 callback 基础，remote 默认路径仍需 host contract 接通 |
| `reading.progress.update` | Core | V1 已完成 | `crates/reader-storage/src/lib.rs` |
| Chapter/content cache | Core | 部分完成 | 需要持久化和 corpus 证明 |

## Rule、JS、Content

| 能力 | Owner | 状态 | 证据 |
| --- | --- | --- | --- |
| Regex extract/replace | Core | 已完成兼容子集 | `crates/reader-rule/tests/*.rs` |
| JSONPath lookup | Core | 已完成兼容子集 | `crates/reader-rule/tests/rule_edgecases.rs` |
| CSS selector text/attr | Core | 已完成兼容子集 | `crates/reader-rule/tests/*.rs` |
| CSS `:contains` / `:containsOwn` | Core | 已完成基础用例 | `crates/reader-rule/src/lib.rs` |
| XPath extraction 与 namespace | Core | 已完成兼容子集 | `crates/reader-rule/tests/rule_edgecases.rs` |
| Rule chaining/fallback | Core | 已完成基础能力 | `crates/reader-rule/src/lib.rs` |
| Legado rule 全量兼容 | Core | Gap | 需要 Legado ledger 和 corpus runner |
| QuickJS evaluation | Core | 已完成基础 sandbox | `crates/reader-js/src/lib.rs` |
| JS timeout/cancel/console/host callback registry | Core | 已完成基础能力 | `crates/reader-js/src/lib.rs` |
| JS host callback 与 runtime host bus | Core + host | Gap | 需要 request/session contract 接通 |
| HTML/XML/JSON content extraction | Core | 部分完成 | V1 fixture 有覆盖，未达全量 corpus parity |

## Storage 与 data

| 能力 | Owner | 状态 | 证据 |
| --- | --- | --- | --- |
| Source registry | Core | 已完成基础能力 | `crates/reader-storage/src/lib.rs` |
| Book cache | Core | 已完成基础能力 | `crates/reader-storage/src/lib.rs` |
| Chapter cache | Core | 已完成基础能力 | `crates/reader-storage/src/lib.rs` |
| Reading progress | Core | 已完成基础能力 | `crates/reader-storage/src/lib.rs` |
| Snapshot import/export | Core | 已完成基础能力 | storage/local/RSS/sync crates |
| SQLite schema 与 migration | Core | Gap | 需要持久化设计和 gate |
| Cookie/session persistence | Core protocol + host capture | Gap | 需要 session ledger 和 host proof |
| Download queue/recent history/offline cache | Core | Gap | 需要 RECOVERY-33 迁移 |

## 本地书、RSS、同步

| 能力 | Owner | 状态 | 证据 |
| --- | --- | --- | --- |
| TXT parsing | Core | 部分完成 | `crates/reader-local-book/src/lib.rs`、`src/txt.rs` |
| 本地书 library snapshot | Core | 部分完成 | `crates/reader-local-book/src/lib.rs` |
| EPUB/PDF/MOBI/UMD | Core/策略 | Gap | 需要支持策略或明确 policy error |
| RSS parse/subscription state | Core | 部分完成 | `crates/reader-rss/src/lib.rs` |
| WebDAV/sync/backup/conflict model | Core semantics + host transport | 部分完成 | `crates/reader-sync/src/lib.rs` |

## 平台与宿主能力

| 能力 | Owner | 状态 | 证据 |
| --- | --- | --- | --- |
| Real HTTP/TLS/socket execution | Host/app | Host/app | Core 只发 `http.execute` |
| WebView login/captcha/cookie capture | Host/app | Host/app | Core 不实现 WebView UI |
| Secure credentials | Host/app | Host/app | Keychain/Keystore/Secure Store 属于平台 |
| File picker 与 sandbox grants | Host/app | Host/app | 文件权限 UI 属于平台 |
| TTS audio playback | Host/app | Host/app | Core 不包含媒体播放 |
| UI/navigation/theme/font/background/notification | Host/app | Host/app | 不属于 Core tree |

## 平台 binding lane

| 能力 | Owner | 状态 | 证据 |
| --- | --- | --- | --- |
| iOS XCFramework/header/modulemap smoke | Core | 已完成 smoke | `scripts/build-ios-xcframework.sh`、`bindings/ios/README.md` |
| iOS Swift wrapper smoke | Core | 已完成 lifecycle/info/ping | `bindings/ios/Sources/ReaderCoreClient/ReaderCoreClient.swift` |
| Android JNI wrapper | Core wrapper | 已合并，需 NDK/device proof | `bindings/android/**`、`build-android-jni.sh` |
| Harmony NAPI wrapper | Core wrapper | 已合并，需 HAP/device proof | `bindings/harmony/**`、`scripts/build-harmony-napi.sh` |

## 不能过度声明的事项

- wrapper compile/link smoke 不等于 App/device parity。
- CLI fixture vertical 不等于 Legado 全量兼容。
- data snapshot 测试不等于三端持久化迁移完成。
- 没有 corpus canonical hash，就不能把能力计为 parity complete。

最后更新：2026-06-25。
