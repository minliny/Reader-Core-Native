# Android JNI SDK 集成状态

> 当前最高优先级文档是 `docs/LOCAL_REPO_MIGRATION_DIRECTIVE.md`。本文只记录 JNI
> wrapper 状态，不代表 Android App 迁移完成。

最后更新：2026-06-25

基线：由 `codex/reader-core-runtime-protocol`、
`codex/reader-core-c-abi-stable-boundary`、`codex/android-jni-sdk` 整合而来。

范围规则：Android 文件只消费 C ABI；Android wrapper 工作不能设置 Core 语义。

## 已闭环能力

| 能力 | 实现 | 当前覆盖 |
| --- | --- | --- |
| Runtime create / destroy | `ReaderCoreRuntime` 通过 JNI handle ownership 调用 `rc_runtime_create` / `rc_runtime_destroy` | Java wrapper 与 native lifecycle |
| 发送 command | `ReaderCoreRuntime.send` / `sendCommand` 调用 `rc_runtime_send` | Java/Kotlin compile gate、native C++ syntax gate、NDK build gate |
| Poll / parse event | JNI callback 把 event bytes 复制到 native queue；Java 轮询 `byte[]` 或 UTF-8 `String` | Java wrapper 与 samples |
| Cancel | `ReaderCoreRuntime.cancel` 调用 `rc_runtime_cancel` | Java wrapper |
| `host.complete` / `host.error` | Java helper 组装 protocol JSON 并通过 `rc_runtime_send` 发送 | Java/Kotlin sample 从 `host.request` 解析 `operationId` 后完成 |
| CMake / NDK build | `bindings/android/jni/CMakeLists.txt` 与 `build-android-jni.sh` | `arm64-v8a` 和 `x86_64` 真实 NDK link 已通过 |
| Java/Kotlin 最小调用 | `bindings/android/samples/**` 与 direct listener sample | Java compile gate；`kotlinc` 可用时 Kotlin compile gate |
| Direct listener API | 单一 Java `NativeCoreBridge` class 加 Kotlin `ReaderEventListener` | 已整合，避免重复 `com.reader.core.NativeCoreBridge` |

## 本地验证记录（2026-06-24）

已通过：

- `git diff --check -- bindings/android build-android-jni.sh`
- `bash -n build-android-jni.sh`
- `javac --release 8` 编译 Java wrapper 与 Java sample
- Android Studio bundled `kotlinc -jvm-target 1.8` 编译 Kotlin sample
- `c++ -std=c++17 -fsyntax-only` 检查 `bindings/android/jni/reader_jni.cpp`
- `./build-android-jni.sh`
  - Java wrapper/sample gate：passed
  - Kotlin sample gate：passed
  - Rust `reader-ffi` staticlib cross-build：`aarch64-linux-android` 与
    `x86_64-linux-android` passed
  - CMake/NDK JNI shared-library link：`arm64-v8a` 与 `x86_64` passed
- 输出验证：
  - `target/android-jni/libs/arm64-v8a/libreader_core_jni.so`：ELF64 AArch64
  - `target/android-jni/libs/x86_64/libreader_core_jni.so`：ELF64 x86-64
  - 两个 `.so` 均导出 `Java_com_reader_core_NativeCoreBridge_native*` JNI entry point
    和 bridge 使用的 Core ABI symbol

合并分支说明：

- JNI source 现在还导出 direct listener entry point：`pingSmoke`、`runtimeCreate`、
  `runtimeSend`、`runtimeCancel`、`runtimeDestroy`。
- 上述验证来自原 lane。整合分支在作为 Android release evidence 前，仍需要重新跑
  一次新鲜 NDK build。

当时安装的 toolchain：

- Android SDK CMake `3.22.1`
- Android NDK `26.3.11579264`
- Rust target：`aarch64-linux-android`、`x86_64-linux-android`

## Host adapter 接入路径（2026-06-25 新增）

`bindings/android/host-adapter/` 是纯 JVM host 适配器模块，桥接 Core
`host.request` event 到 host capability，并编码 `host.complete` / `host.error`
command 经现有 `rc_runtime_send` 通道回 Core。**不触碰 C ABI**，只消费协议。

- 组件：`HostBus` / `HostEventLoop` / `HostTransport` / `ReaderCoreHostTransport` /
  `HostRequest` / `HostReply` / `CapabilityHandler` / `HostAdapter` /
  `HostReplyCodec` / `HttpExecuteHandler` / `HttpFetch` / `HttpRequest` /
  `HttpResponse` / `HostSmokeEchoHandler` / `CredentialResolveHandler` /
  `CredentialProvider` / `Credential` / 零依赖 `Json`。
- 闭环：`HostEventLoop.tick` 做 poll → 过滤 `host.request` → dispatch → encode →
  send；`ReaderCoreHostTransport` 把 loop 接到现有 `ReaderCoreRuntime`（JNI → C ABI）。
- 产品 surface：`HostBus.over(transport).register(cap, handler).start()/stop()` 把
  transport + adapter + loop 打包成 host app 的一站式接入点，含 daemon 轮询线程与
  同步 `tick`/`drain` 脚本入口。
- 真实 capability：`HttpExecuteHandler`（`http.execute` shared-contract）、
  `HostSmokeEchoHandler`（`host.smoke.echo` conformance smoke）、
  `CredentialResolveHandler`（`credential.resolve`，填补 host-app-contracts Gap D）。
  host-owned 机制（`HttpFetch` / `CredentialProvider`）可注入，TLS/socket/keystore 留 host 侧。
- 命令/响应半边：`HostCommander` 发送 Core command 并按 `requestId` 关联等待
  `result`/`error` event，返回 `CommandResult`（success/error/timeout）——host app
  发起命令的协议入口，与 `HostEventLoop`（应答 host.request）互补。
- 统一 facade：`HostRuntime` 单一 poll 线程 demultiplex 事件流——`host.request` →
  adapter 应答，`result`/`error` → pending `sendAndAwait` future，解决 HostCommander
  的 demultiplexing caveat，给 host app 一个并发安全双向入口。
- Gradle gate：`bindings/android/host-adapter` 下 `gradle check`（JDK 17、
  Gradle 9.5.1 验证；`--offline` 可复跑）。模块通过 `sourceSets` 编译引用现有
  Java JNI wrapper，不修改 wrapper 源。76 unit tests pass（含 10 个上游 fixture 动态扫描）+ `compileSample` gate。
- Contract evidence：`HostReplyCodecTest` / `HostEventLoopTest` /
  `HttpExecuteHandlerTest` / `HostBusTest` 用拷贝/上游 fixture 验证；
  `ProtocolConformanceTest` **直接读取上游 `protocol/fixtures/conformance/host/`**，
  协议变更即断测；`CredentialResolveHandlerTest` 用 fake `CredentialProvider` 验证
  Gap D 草案契约（未知 handle → 非重试 INTERNAL，provider 抛异常 → 可重试 INTERNAL）。
- Contract evidence：`HostReplyCodecTest` / `HostEventLoopTest` /
  `HttpExecuteHandlerTest` 用拷贝 fixture 做单元级验证；`ProtocolConformanceTest`
  **直接读取上游 `protocol/fixtures/conformance/host/`**（经 Gradle system property
  注入路径），断言 codec 输出与 `complete/error/http-complete-with-metadata/
  http-complete-invalid-status` fixture 逐字节一致（modulo outbound requestId），
  拒绝 `complete/error-operation-zero` 负 fixture，并校验 `request.json` 的 smoke
  参数形状与 invalid-capability 负 fixture —— 协议变更即断测。
- 模块详情见 `bindings/android/host-adapter/README.md`。

## ABI gap 记录

1. **Event 只通过 callback 传递。** ABI v1 没有 `rc_runtime_poll`，因此 JNI 层拥有
   native event queue，并从 `rc_event_callback` 填充。
2. **Host completion 是 protocol-level。** ABI v1 没有 `rc_host_complete`；
   Android 通过 `rc_runtime_send` 发送 `host.complete` / `host.error` JSON command。
3. **整合后的 C ABI 已有 structured last-error，但 Android 尚未暴露。**
   `include/reader_core.h` 已暴露 `rc_last_error`。Android 在后续 wrapper 变更前仍用
   `ReaderCoreException` 报告 coarse status。
4. **不声明 Android App project 完成。** 本 lane 只提供 JNI library、Java wrapper 和
   Java/Kotlin minimal sample。Gradle packaging、Android UI、WebView、CookieManager、
   keystore、network policy integration 仍属于 host-app 工作。

## 线程说明

- Core 从 Core-owned background thread 调用 `rc_event_callback`。queue-style Java
  wrapper 会立即复制 event bytes。direct listener API 会从该 Core-owned thread 调用
  `ReaderEventListener.onEvent`，listener implementation 必须 thread-safe。
- `ReaderCoreRuntime.pollEvent(...)` 会阻塞调用线程直到 event 到达或 timeout。长等待
  不应在 UI thread 调用。
- `ReaderCoreRuntime` 在 Java object 上同步 lifecycle、send、cancel、poll。需要更高
  吞吐的 host 可在此 minimal wrapper 之上增加 dispatcher，不需要改 ABI。
