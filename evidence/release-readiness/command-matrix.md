# 命令矩阵

> 基线 `fb4c3a7`。所有命令在 macOS host 执行（除非另注）。
> “gate”列：该命令是否适合作为 release gate（即 CI/发布前必跑且结果可判定）。
> round-01 全部命令在本轮实跑并记录（见 [rounds/2026-06-25-round-01.md](rounds/2026-06-25-round-01.md)）。

## A. 本地 Core gate（scripts/check-local.sh 拆解）

| 命令 | 作用 | 平台 | round-01 结果 | 失败原因 | release gate |
|------|------|------|---------------|----------|--------------|
| `cargo fmt --check` | 代码格式检查 | macOS host | PASS（exit 0） | — | 是（格式 gate） |
| `cargo test --workspace` | 全 workspace 单测+集成测试 | macOS host | PASS，92 passed / 0 failed | — | 是（测试 gate） |

`scripts/check-local.sh` = `cargo fmt --check` + `cargo test --workspace`，等价于上两行。

## B. 本地构建+ABI smoke（scripts/build-local.sh 拆解）

| 命令 | 作用 | 平台 | round-01 结果 | 失败原因 | release gate |
|------|------|------|---------------|----------|--------------|
| `cargo build --workspace` | 全 workspace debug 构建 | macOS host | PASS（由 `cargo test --workspace` 隐式覆盖编译） | — | 是（构建 gate） |
| `cargo build -p reader-ffi --release` | release 静态库 | macOS host | PASS（由 ffi-smoke.sh 覆盖） | — | 是（release 构建 gate） |
| `cargo run -p reader-cli`（默认 `--info`） | ABI smoke：core.info 返回 abiVersion=1 + 9 capabilities | macOS host | PASS | — | 是（ABI smoke gate） |
| `./scripts/ffi-smoke.sh` | C + C++ 链接 `libreader_core.a` 并跑 runtime.ping | macOS host | PASS，C/C++ 二进制均 `pong:true`（host FFI smoke，**非** App/device） | — | 是（host FFI ABI smoke gate） |

`scripts/build-local.sh` = 上四步组合。本轮未整脚本跑，但各分量均实跑覆盖。

## C. 协议一致性 + 垂直 smoke

| 命令 | 作用 | 平台 | round-01 结果 | 失败原因 | release gate |
|------|------|------|---------------|----------|--------------|
| `cargo run -p reader-cli -- --conformance` | 协议 conformance（命令/配置/host/cancel 共 18 例） | macOS host | PASS，18/18 passed | — | 是（协议 conformance gate） |
| `cargo run -p reader-cli -- --fixture-vertical tests/fixtures/remote_source/basic_source.json` | remote.reading.v1 垂直 smoke：import→search→host.http.execute→detail→toc→content→progress→JS-unsupported-error | macOS host | PASS，9 步全通（requestId 9 正确返回结构化 `JS rule unsupported in V1` 错误） | — | 是（Core-side 垂直 smoke gate） |

## D. 平台交叉构建（Core-side 产物）

| 命令 | 作用 | 平台 | round-01 结果 | 失败原因 | release gate |
|------|------|------|---------------|----------|--------------|
| `./scripts/check-ios-swift-wrapper.sh` | 构建 iOS xcframework（device+sim）+ swift typecheck + host sim Swift wrapper smoke（core.info/runtime.ping） | iOS（xcframework）+ macOS host sim | PASS，xcframework 生成、`ReaderCoreClient.swift` typecheck ok、`swift client smoke passed`（host sim wrapper smoke，**非** iOS App/device） | — | 是（Core-side iOS 产物 + wrapper 编译/链接/运行 smoke gate）；**否** for App/device |
| `./scripts/build-ios-xcframework.sh` | 仅构建 xcframework + sim swift typecheck | iOS | PASS（由 check-ios-swift-wrapper.sh 内部调用覆盖） | — | 是（Core-side iOS 产物 gate） |
| `./scripts/build-ohos.sh` | OHOS 交叉构建 `reader-ffi` release 静态库 | HarmonyOS/OHOS（aarch64-unknown-linux-ohos） | PASS，`target/aarch64-unknown-linux-ohos/release/libreader_core.a` 生成（由 build-harmony-napi.sh 内部调用覆盖） | — | 是（Core-side OHOS 产物 gate） |
| `./scripts/build-harmony-napi.sh` | OHOS 静态库 + NAPI `.so`（CMake/Ninja 链接 `reader_napi.cpp`） | HarmonyOS/OHOS | PASS，`target/harmony-napi/arm64-v8a/libreader_core_napi.so` 生成（Core-side NAPI `.so` 构建 smoke，**非** HAP/device） | — | 是（Core-side NAPI `.so` 构建 gate）；**否** for HAP/device |

## E. 集成队列（人工触发，非常规 gate）

| 命令 | 作用 | 平台 | round-01 结果 | 失败原因 | release gate |
|------|------|------|---------------|----------|--------------|
| `./scripts/integration-queue.sh <branch> <base> <src...>` | 兄弟 worktree 合并多 agent 分支并跑 check-local.sh + build-local.sh（可选 OHOS/NAPI） | macOS host | 未跑（人工集成工具，非自动 gate） | — | 否（集成工具，按需触发） |

## F. CI（.github/workflows/core.yml，仅参考，不在本证据包修改范围）

CI 在 `macos-15` runner 上跑：check-local.sh、build-local.sh、`cargo run -p reader-cli -- --conformance`、`rustup target add aarch64-apple-ios aarch64-apple-ios-sim`、check-ios-swift-wrapper.sh。与本地面板 B/C/D 一致。CI 修改属禁止范围，仅作对照。

## 非致命告警（记录，不阻塞）

- host FFI smoke 与 iOS swift wrapper smoke 链接时出现 `ld: warning: object file ... was built for newer 'macOS' version (26.5) than being linked (26.0)`（QuickJS C 对象版本高于 host 链接目标）。仅告警，不影响产物与 smoke 结果。
