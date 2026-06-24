# 环境依赖

> round-01 实采环境（macOS host）。验证日期 2026-06-25（Asia/Shanghai）。

## 工具链

| 项 | 值 | 来源 |
|----|----|------|
| rustc | 1.96.0 (ac68faa20 2026-05-25) | `rust-toolchain.toml` channel=stable |
| cargo | 1.96.0 (30a34c682 2026-05-25) | 同上 |
| active toolchain | stable-aarch64-apple-darwin | rustup show |
| MSRV（声明） | 1.75 | `Cargo.toml` rust-version |
| default host | aarch64-apple-darwin | rustup show |

## 已安装 Rust target（跨平台构建所需）

- aarch64-apple-darwin（host）
- aarch64-apple-ios（iOS device）
- aarch64-apple-ios-sim（iOS 模拟器）
- aarch64-unknown-linux-ohos（HarmonyOS/OHOS）
- aarch64-linux-android（Android，**已装但无脚本使用**）
- x86_64-linux-android（Android，**已装但无脚本使用**）

## 宿主 OS / 工具

| 项 | 值 |
|----|----|
| OS | macOS 26.5.1（Build 25F80） |
| kernel | Darwin 25.5.0 arm64 |
| Xcode | 26.5（Build 17F42） |
| xcodebuild | 可用（xcframework + swiftc 所需） |
| cc/c++ | clang（ffi-smoke.sh 所需） |

## SDK 依赖

| 变量 | 值 | 用途 |
|------|----|------|
| `OHOS_SDK_HOME` | `/Applications/DevEco-Studio.app/Contents/sdk/default` | `build-ohos.sh` + `build-harmony-napi.sh` 必需 |
| `LIBCLANG_PATH`（默认） | `/Library/Developer/CommandLineTools/usr/lib` | OHOS 交叉构建 bindgen 所需（脚本默认值，未显式设置时使用） |

OHOS 构建还需 SDK 内 `openharmony/native` 下：`build/cmake/ohos.toolchain.cmake`、
`build-tools/cmake/bin/cmake`、`build-tools/cmake/bin/ninja`、`llvm/bin/clang`、
`sysroot`。round-01 均存在。

## 各命令的环境前提

| 命令 | 前提 |
|------|------|
| `cargo fmt --check` / `cargo test --workspace` / `cargo run -p reader-cli ...` | rust stable toolchain |
| `./scripts/ffi-smoke.sh` | rust + cc/c++（clang） |
| `./scripts/build-ios-xcframework.sh` | rust + iOS targets + `xcodebuild` |
| `./scripts/check-ios-swift-wrapper.sh` | 同上 + swiftc（Xcode 自带） |
| `./scripts/build-ohos.sh` | rust + `aarch64-unknown-linux-ohos` target + `OHOS_SDK_HOME` + libclang |
| `./scripts/build-harmony-napi.sh` | 同上 + SDK 内 CMake/Ninja |
| Android 构建 | **无脚本**；target 已装但无构建路径 |

## 缺失/可选

- Android：无构建脚本、无 JNI 桥，环境齐备也无法验证 Android Core-side 产物。
- CI runner `macos-15` 与本地 macOS 26.5.1 不同；CI 结果未采集。
