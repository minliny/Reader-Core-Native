# Reader-Core-Native 架构

本文描述当前分支 HEAD 的代码架构。全量产品目标和开发路线见
`docs/FULL_DEVELOPMENT_ROADMAP.md`。

本文只在能力有当前仓库代码路径、验证命令，或可由 Git 证明的分支事实时，才将
能力标记为完成。路线级 parity 声明仍必须满足全量路线中定义的 Legado 能力账本、
Reader-Core 迁移账本、Native/C ABI 证据和 corpus benchmark 证明。

## 当前基线

当前分支基线来自 `codex/full-branch-directory-consolidation`。它已经合并此前分散的
runtime/protocol、C ABI、Android JNI、Harmony NAPI、iOS wrapper、data subsystem、
rule/JS、corpus、CI/evidence 文档等工作。

已验证的主 gate：

```bash
git diff --check
cargo run -p reader-cli -- --conformance
./scripts/ffi-smoke.sh
cargo test --workspace
```

已知验证结果记录见 `reports/full-consolidation/2026-06-25.md`。

## Runtime 形态

```text
Platform app
  -> Swift / JNI / NAPI host wrapper
  -> include/reader_core.h C ABI
  -> crates/reader-ffi
  -> crates/reader-runtime worker
  -> crates/reader-contract command/event DTOs
  -> domain, rule, JS, content, storage modules
  -> host.request events for platform-owned capabilities
```

ABI 暴露一个 opaque runtime handle，并通过 JSON command/event 交换消息。

主要证据：

- `include/reader_core.h`
- `crates/reader-ffi/src/lib.rs`
- `crates/reader-ffi/src/runtime.rs`
- `crates/reader-runtime/src/runtime.rs`

ABI v1 导出函数：

- `rc_abi_version`
- `rc_runtime_create`
- `rc_runtime_send`
- `rc_runtime_cancel`
- `rc_runtime_destroy`

ABI v1 没有 `rc_buffer_free`。事件 callback 中的 buffer 只在 callback 生命周期内
借用；宿主如需保存，必须复制 bytes。

## Protocol 与 capability

Protocol v1 由 `crates/reader-contract` 和 `protocol/` 中的 JSON schema 表示。

证据：

- `crates/reader-contract/src/lib.rs`
- `crates/reader-contract/src/command.rs`
- `crates/reader-contract/src/event.rs`
- `protocol/reader-command.schema.json`
- `protocol/reader-event.schema.json`
- `protocol/compatibility.md`

当前 capability：

- `core.info`
- `runtime.ping`
- `runtime.hostSmoke`
- `runtime.status`
- `runtime.shutdown`
- `runtime.cancel`
- `host.complete`
- `host.error`
- `host.bus.v1`
- `http.execute`
- `runtime.config.v1`
- `remote.reading.v1`

`core.ping` 只作为 bootstrap alias 被 runtime dispatch 接受，不作为正式 capability
对外声明。

## 已实现的 Core-owned 能力

| 能力 | 当前状态 | 证据 |
| --- | --- | --- |
| Command/event protocol v1 | 已实现 | `crates/reader-contract/src/*.rs`、`protocol/*.schema.json`、`cargo run -p reader-cli -- --conformance` |
| Runtime worker、request id、重复 active 拒绝、cancel | 已实现 | `crates/reader-runtime/src/runtime.rs`、`cargo test -p reader-runtime` |
| Runtime status/shutdown | 已实现 | `protocol/fixtures/conformance/commands/*runtime-status*`、`*runtime-shutdown*` |
| Host bus request/complete/error 路由 | 已实现 | `crates/reader-runtime/src/runtime.rs`、`protocol/fixtures/conformance/host/*.json` |
| C ABI lifecycle | 已实现 | `include/reader_core.h`、`crates/reader-ffi/src/*.rs`、`./scripts/ffi-smoke.sh` |
| Structured last-error 与 panic guard | 已实现 | `crates/reader-ffi/src/last_error.rs`、`crates/reader-ffi/src/panic_guard.rs` |
| Rule primitives | 已实现一个兼容子集 | `crates/reader-rule/src/lib.rs`、`crates/reader-rule/tests/*.rs` |
| QuickJS sandbox | 已实现基础能力 | `crates/reader-js/src/lib.rs`、`cargo test -p reader-js` |
| Remote-reading V1 vertical | 已实现 Core-side 纵切 | `crates/reader-runtime/src/remote.rs`、`tools/reader-cli/tests/fixture_vertical.rs` |
| HTTP transport contract | 已实现 host capability contract | `crates/reader-contract/src/remote.rs`、`crates/reader-runtime/src/remote.rs` |
| Data/storage snapshot | 已实现基础状态机 | `crates/reader-storage/src/lib.rs` |
| TXT local book | 已实现基础解析和 library snapshot | `crates/reader-local-book/src/lib.rs`、`crates/reader-local-book/src/txt.rs` |
| RSS state | 已实现基础 parse/subscription state | `crates/reader-rss/src/lib.rs` |
| Sync package/journal | 已实现基础 merge/backup model | `crates/reader-sync/src/lib.rs` |
| iOS wrapper smoke | 已实现 ABI lifecycle、`core.info`、`runtime.ping` | `bindings/ios/**`、`scripts/check-ios-swift-wrapper.sh` |
| Android JNI wrapper shape | 已合并 | `bindings/android/**`、`build-android-jni.sh` |
| Harmony NAPI wrapper shape | 已合并 | `bindings/harmony/**`、`scripts/build-harmony-napi.sh` |

## 已知 gap

| Gap | Owner | 说明 |
| --- | --- | --- |
| Legado 全量能力账本 | Core/规划 | 尚未形成 source-backed ledger |
| 旧 Reader-Core 迁移账本 | Core/规划 | 尚未系统映射 migrate/replay/host/archive |
| runtime config 通过 C ABI create path 完整接入 | Core | schema 和 Rust config 已存在，仍需持续核对 ABI 创建路径 |
| SQLite-backed persistent store 和 migration | Core | 现有 storage 更偏确定性状态机和 snapshot，需补持久化策略 |
| EPUB/PDF/MOBI/UMD 支持 | Core/策略 | 当前 TXT 有基础实现，其余格式需明确支持或 policy error |
| Full Legado rule/request/JS parity | Core | 需要能力账本和 corpus runner 证明 |
| JS network 与 WebView-only 行为 | Core + host | sandbox 有 host callback 基础，WebView/login/captcha 必须由 host contract 表达 |
| Real HTTP/TLS/socket | Host/app | Core 只发出 `http.execute`，平台执行网络 |
| WebView login、captcha、cookie extraction | Host/app | Core 不内置 WebView UI |
| Keychain/Keystore/Secure Store | Host/app | Core 不持有平台安全存储实现 |
| File picker 和 sandbox grants | Host/app | Core 不实现平台权限 UI |
| TTS、notification、UI/navigation/theme | Host/app | 不属于 Core |
| App/device proof | Host/app | wrapper smoke 不等于 App/device 完成 |
| 跨平台 corpus benchmark | Core + host | 仍需 CLI/iOS/Android/Harmony canonical result 对比 |

## Core-owned 与 host-owned 边界

Core 负责确定性产品语义：

- Protocol DTO、method name、capability advertisement、structured error。
- Rule execution、content extraction、QuickJS sandbox 行为。
- Remote-reading state machine 与 host operation continuation。
- Storage/local/RSS/sync 的数据语义、snapshot、migration、hash。
- Source、book、toc、chapter、progress domain model。

Host/app 负责平台能力与用户体验：

- HTTP/TLS/socket 执行与平台网络策略。
- WebView login、captcha、cookie capture、platform session store。
- 安全凭据存储。
- File picker、sandbox grants、document access。
- TTS playback、media session、notification。
- Background execution、app lifecycle、UI、navigation、theme。
- Packaging、signing、crash reporting、store distribution。

当前边界通过 host bus 体现：Core 发出 `host.request`，宿主以 `host.complete`
或 `host.error` 回复。

## 验证命令

Core-local gate：

```bash
cargo fmt --check
cargo test --workspace
cargo run -p reader-cli -- --conformance
cargo run -p reader-cli -- --host-smoke
cargo run -p reader-cli -- --fixture-vertical tests/fixtures/remote_source/basic_source.json
./scripts/ffi-smoke.sh
./scripts/build-local.sh
```

平台 smoke gate：

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
./scripts/check-ios-swift-wrapper.sh

rustup target add aarch64-linux-android
./scripts/build-android-jni.sh

OHOS_SDK_HOME=/path/to/ohos-sdk ./scripts/build-harmony-napi.sh
```

## 与全量路线的关系

本文件说明“当前代码长什么样”。是否达到 Legado parity、Core parity 或 production
ready，只能由 `docs/FULL_DEVELOPMENT_ROADMAP.md` 中要求的 ledger、migration、
ABI、platform proof 和 corpus benchmark 共同判定。
