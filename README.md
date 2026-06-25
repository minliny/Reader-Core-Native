# Reader-Core-Native

> Reader 三端统一 Rust 业务内核。
>
> 当前最高优先级文档是
> [docs/LOCAL_REPO_MIGRATION_DIRECTIVE.md](./docs/LOCAL_REPO_MIGRATION_DIRECTIVE.md)。
> 全量开发路线见
> [docs/FULL_DEVELOPMENT_ROADMAP.md](./docs/FULL_DEVELOPMENT_ROADMAP.md)。

## 当前定位

目标 Rust 仓库统一使用 `Reader-Core-Native`。当前可操作路径为：

```text
/Users/minliny/Documents/Reader-Core-Native
```

相关本地仓库：

- `/Users/minliny/Documents/Reader-Core`
- `/Users/minliny/Documents/Reader for iOS`
- `/Users/minliny/Documents/Reader for Android`
- `/Users/minliny/Documents/Reader for HarmonyOS`

本地仓库是唯一事实来源。远程 README、历史讨论、旧规划文档只能作为补充。

## 快速验证

```bash
# 本机检查
./scripts/check-local.sh

# 本机构建 Rust workspace、FFI release 产物，并运行 C/C++ ABI smoke
./scripts/build-local.sh

# 协议一致性
cargo run -p reader-cli -- --conformance

# iOS XCFramework / Swift wrapper smoke（需要 Xcode）
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
./scripts/check-ios-swift-wrapper.sh

# Android JNI smoke module（需要 Android NDK）
rustup target add aarch64-linux-android
./scripts/build-android-jni.sh

# HarmonyOS NAPI smoke module（需要 DevEco/OHOS SDK）
rustup target add aarch64-unknown-linux-ohos
./scripts/build-harmony-napi.sh
```

## 开工前必须执行

```bash
pwd
find .. -maxdepth 2 -type d -name .git
git -C <repo> status --short
git -C <repo> branch --show-current
git -C <repo> log -5 --oneline
```

至少检查：

- 旧核心：`Reader-Core`
- iOS：`Reader for iOS`
- Android：`Reader for Android`
- HarmonyOS：`Reader for HarmonyOS`
- Rust 目标仓库：`Reader-Core-Native`

## 目录

- [docs/LOCAL_REPO_MIGRATION_DIRECTIVE.md](./docs/LOCAL_REPO_MIGRATION_DIRECTIVE.md) — 当前迁移指令和安全要求
- [docs/FULL_DEVELOPMENT_ROADMAP.md](./docs/FULL_DEVELOPMENT_ROADMAP.md) — 全量开发路线
- [ARCHITECTURE.md](./ARCHITECTURE.md) — 当前 Rust Core 架构
- [FEATURE_MATRIX.md](./FEATURE_MATRIX.md) — 能力状态矩阵
- [MIGRATION_MAP.md](./MIGRATION_MAP.md) — 三端迁移地图
- [docs/ROLLING_INTEGRATION.md](./docs/ROLLING_INTEGRATION.md) — 并行分支和滚动集成规则
- [include/reader_core.h](./include/reader_core.h) — C ABI 头文件
- [protocol/](./protocol/) — JSON command/event schema
- [bindings/ios/README.md](./bindings/ios/README.md) — iOS Swift wrapper
- [bindings/android/README.md](./bindings/android/README.md) — Android JNI wrapper
- [bindings/harmony/README.md](./bindings/harmony/README.md) — HarmonyOS Node-API wrapper

## 架构边界

Rust Reader-Core 是唯一业务内核，负责规则、JS、书源、阅读链路、数据模型、缓存、
同步、恢复、diff 和跨平台协议。

iOS、Android、HarmonyOS 只负责平台能力：

- UI / navigation / theme
- HTTP 实际执行
- WebView 登录、Cookie、captcha、DOM
- Keychain / Keystore / credential store
- 文件权限和系统目录
- TTS、通知、后台任务
- 打包、签名、分发

wrapper smoke 不等于 App/device 完成。三端一致性必须由同一个 Rust Core commit 和
相同 fixture/corpus 的 canonical result 证明。
