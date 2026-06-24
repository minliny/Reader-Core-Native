# 平台矩阵

> 基线 `fb4c3a7`。区分三层：**Core-side 产物构建**、**host smoke**、**App/device 真机**。
> smoke 通过 ≠ App/device 通过。

| 平台 / 目标 | Core-side 产物构建 | host smoke | App/device 真机 | round-01 状态 |
|-------------|--------------------|------------|-----------------|---------------|
| **macOS host**（aarch64-apple-darwin） | `cargo build --workspace` / `reader-ffi --release` 静态库 | fmt / test(92) / conformance(18) / vertical(9 步) / ffi-smoke(C/C++ pong) / cli `--info` ABI | N/A（host 不是发布目标） | ✅ Core-side 全绿 |
| **iOS**（aarch64-apple-ios + aarch64-apple-ios-sim） | `build-ios-xcframework.sh`：`ReaderCore.xcframework`（device+sim 静态库 + headers + modulemap） | `check-ios-swift-wrapper.sh`：swift typecheck + host sim Swift wrapper smoke（core.info/runtime.ping，链接 debug `libreader_core.a`） | ❌ 未做 iOS App 构建/真机/模拟器 App 启动 | ✅ 产物+wrapper smoke；❌ App/device |
| **HarmonyOS / OHOS**（aarch64-unknown-linux-ohos） | `build-ohos.sh`：`libreader_core.a`（release 交叉静态库）；`build-harmony-napi.sh`：`libreader_core_napi.so`（CMake/Ninja 链接 `reader_napi.cpp`） | Core-side NAPI `.so` 构建 smoke（产物存在性 + 链接成功） | ❌ 未做 HAP 打包/真机/DevEco 启动 | ✅ 产物+.so 构建 smoke；❌ HAP/device |
| **Android**（aarch64-linux-android、x86_64-linux-android） | ❌ 无构建脚本，`bindings/android` 仅 `.gitkeep`；Rust target 已安装但未被任何脚本使用 | ❌ 无 | ❌ 无 | ❌ 不可声明 |

## 平台说明

- **macOS host**：仅作为构建/验证宿主，不是发布目标平台。其“全绿”只证明 Core 在 host
  上构建/测试/ABI smoke 通过。
- **iOS**：`bindings/ios/Sources/ReaderCoreClient/ReaderCoreClient.swift` 为 Swift wrapper；
  round-01 仅验证 wrapper 编译+链接+host sim runtime.ping/core.info smoke。**未**构建 iOS
  App、**未**在模拟器/真机启动 App、**未**验证 host adapter 接入。
- **HarmonyOS**：`bindings/harmony/native/reader_napi.cpp` 为 NAPI 桥；round-01 仅验证
  `.so` 产物链接成功。**未**打 HAP、**未**真机/DevEco 启动、**未**验证 `Index.ets`/SDK TS
  侧（`bindings/harmony/sdk/*.ts` 存在但未跑）。
- **Android**：仓库无 Android 构建脚本与 JNI 桥（`bindings/android` 仅有 `.gitkeep`）。
  Android release readiness 当前**不可声明**，属阻塞项（见 [blockers.md](blockers.md)）。

## 跨平台 host adapter 能力（参考 FEATURE_MATRIX.md，非本证据包验证范围）

以下能力属 platform adapter，Core-side smoke 不覆盖：TLS/真实 socket、HTTP Transport、
系统 TTS、登录 WebView、WebView Cookie、Keychain/Keystore、文件选择/沙箱、UI、后台任务、
包体签名/分发。这些不计入 Core release gate。
