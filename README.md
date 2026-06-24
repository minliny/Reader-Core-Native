# Reader-Core-Native

> **Reader 三平台统一内核 — Rust 实现**
>
> 以 [ARCHITECTURE.md](./ARCHITECTURE.md) 为唯一实施规划来源。

## 快速开始

```bash
# 安装 Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 本机验证
./scripts/check-local.sh

# 本机构建 Rust workspace、FFI release 产物，并运行 Rust/C ABI smoke
./scripts/build-local.sh

# 阶段 1：仅构建 OHOS Rust staticlib
rustup target add aarch64-unknown-linux-ohos
./scripts/build-ohos.sh

# 阶段 1：构建 HarmonyOS NAPI smoke module（需要 DevEco/OHOS SDK）
./scripts/build-harmony-napi.sh

# 阶段 1：构建 iOS XCFramework smoke artifact（需要 Xcode）
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
./scripts/build-ios-xcframework.sh

# 阶段 1：Swift wrapper compile/link/runtime smoke（需要 Xcode）
./scripts/check-ios-swift-wrapper.sh

# 阶段 2：构建 Android JNI smoke module（需要 Android NDK）
rustup target add aarch64-linux-android
./scripts/build-android-jni.sh

# 滚动集成：把已完成 agent 分支接入独立 integration worktree
scripts/integration-queue.sh \
  codex/android-integration \
  origin/codex/core-product-integration \
  origin/codex/<android-jni-branch>
```

`build-local.sh` 会同时运行 C 和 C++ host ABI smoke。C++ smoke 是
JNI、NAPI、Objective-C++ shim 的头文件/链接基线。

## 当前 Core-side 状态

`origin/codex/core-product-integration` 已接入 Core-side
`remote.reading.v1` 纵切 smoke：`source.import`、`book.search`、
`book.detail`、`book.toc`、`chapter.content`、`reading.progress.update` 可在
fixture/inline response 下跑通，并覆盖 content pipeline、in-memory cache
和 progress 写入；同时支持 `http.execute` host request/complete 回路。V1
不在 Core 内打开 socket；HTTP/TLS/WebView 等实际平台能力仍由 platform
adapter 提供。

OHOS、Android、iOS 平台产物脚本会按 [ARCHITECTURE.md](./ARCHITECTURE.md)
阶段 1/2 补齐；当前 `build-harmony-napi.sh` 验证 Rust staticlib 能链接为
HarmonyOS NAPI `.so`，HAP 集成和真机加载仍需在 HarmonyOS App 仓库完成。
当前 iOS 证据覆盖 Core-side XCFramework / Swift wrapper compile-link-runtime
smoke（`core.info` / `runtime.ping`）；URLSession/WebView/App 侧接入仍是后续
滚动接入项。Android lane 已新增 Core-side JNI shim 和
`build-android-jni.sh`，但当前机器缺 Android NDK，`.so` 交叉构建仍需在 NDK
环境验证；App 仓库侧加载和真机集成仍需完成。

## 目录

- [ARCHITECTURE.md](./ARCHITECTURE.md) — 完整架构与实施规划（**唯一权威文档**）
- [FEATURE_MATRIX.md](./FEATURE_MATRIX.md) — 能力归属表
- [MIGRATION_MAP.md](./MIGRATION_MAP.md) — 各平台迁移进度
- [docs/ROLLING_INTEGRATION.md](./docs/ROLLING_INTEGRATION.md) — 并行 agent 滚动集成队列
- [include/reader_core.h](./include/reader_core.h) — C ABI 头文件
- [protocol/](./protocol/) — JSON 消息协议 Schema
- [bindings/android/README.md](./bindings/android/README.md) — Android JNI smoke 产物说明
- [bindings/ios/README.md](./bindings/ios/README.md) — iOS XCFramework smoke 产物说明

## 仓库关系

```
Reader-Core-Native          ← 此仓库：唯一业务内核（Rust）
Reader-for-iOS              ← UI + Apple Host Adapters
Reader-for-Android          ← UI + Android Host Adapters
Reader-for-HarmonyOS        ← UI + Harmony Host Adapters（首个平台验收目标）
Reader-Core (Swift)         ← 归档参考（冻结新功能）
```

## 旧规划文档

各平台仓库中的旧规划/架构/开发计划等文档已在 2026-06-24 统一归档至各自 `_archived_planning_2026-06-24/` 目录，不再作为实施依据。
