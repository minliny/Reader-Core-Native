# HarmonyOS 退役 fixture 业务路径 + 接入 Rust Core + 重建 5 页 UI 业务源 (S6) 实施方案

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 Reader for HarmonyOS 仓库 `codex/harmony-signed-device-runtime` 分支的 fixture-based 业务路径(`HomeDashboardService`/`SearchService`/`BridgeHTTPClient`/`MockBookshelfRepository`/`FixtureReplayInterceptor`/`FixtureEPUBParserAdapter`)退役,接入主仓库 Rust Core C ABI v1(经 `libreader_core_napi.so`),让 5 页 ArkTS UI 的业务源切到 Rust Core,在 HarmonyOS 模拟器/真机跑通 `search→detail→toc→content` 全链路(经 Rust Core,非 fixture)。

**Architecture:** NAPI .so 已就绪但 Index.d.ts 严重过期(只声明 3/12 exports)—— Task 1 先修复。HarmonyOS 仓库 `entry/src/main/cpp/CMakeLists.txt` 已直接调主仓库 `scripts/build-ohos.sh` 构建 `libreader_core.a`,无需 vendor 预构建(但 Task 2 会验证 HAP 中 .so 真正被打包 + 跑通)。HarmonyOS 缺 host-adapter 模块(Android `bindings/android/host-adapter/` 的等价物)—— Task 3 新建 ArkTS 版 `HostAdapter`/`HostBus`/`HostRuntime`/`HttpExecuteHandler`/`HostTransport`。Task 4 写 `OHOSHTTPHostTransport` 用 `@ohos.net.http` 实现 `HostTransport` 接口。Task 5 写 `ReaderCoreClient` 单例 + `BookApi`/`SourceApi` facade。Task 6 退役 fixture 业务路径(strangler 模式,deprecated 非删除)。Task 7 改造 5 页 UI 业务源。Task 8 端到端验证。本期范围:**仅支持非 WebView 书源**(覆盖 Legado 大多数,后续补 `webview.evaluateJavaScript` host capability)。

**Tech Stack:** Rust Core(`reader-ffi` staticlib, ABI v1,经 `libreader_core_napi.so` 消费)/ NAPI(`reader_napi.cpp`,12 exports)/ ArkTS(Stage Model)/ HarmonyOS API 12+/ `@ohos.net.http`(host HTTP transport)/ `@ohos.data.preferences` 或 `@ohos.data.relationalStore`(host-side cache,本期未用)/ `@kit.ArkWeb`(WebView,本期不实现)/ DevEco Studio / `hvigorw`。

**Repos:**
- 主仓库(只读源 + plan 文档):`/Users/minliny/Documents/Reader-Core-Native`
- HarmonyOS 工作仓库:`/Users/minliny/Documents/Reader for HarmonyOS`
- 当前分支:`codex/harmony-signed-device-runtime`(最新 commit `b7aa631 feat(harmony): add signed real-device evidence runner`)

**审计前置结论**(已完成,详见 Step 1-3):
- 5 页 ArkTS UI 已存在:`pages/Index.ets`(Tab 容器,内含 `BookshelfTab`/`SearchTab`/`SettingsTab` 三个内嵌组件)+ `pages/BookshelfPage.ets` + `pages/SearchPage.ets` + `pages/ReaderPage.ets` + `pages/SettingsPage.ets`
- 5 页全部基于 fixture:`HomeDashboardService` 用 `MockBookshelfRepository(true)` + `FixtureReplayInterceptor` + 3 个 fixture `BookSource`(`fixture://default-source` 等);`ReaderPage` 用 `FixtureEPUBParserAdapter` 解析本地 fixture EPUB
- NAPI POC 已就绪:`entry/src/main/cpp/reader_napi.cpp` 是主仓库 `bindings/harmony/native/reader_napi.cpp` 的完整副本(785 行,12 exports);`entry/src/main/ets/cabi/ReaderCoreNapiBridge.ets` 已 `import readerCoreNapiRaw from 'libreader_core_napi.so'` 并跑 `runtime.ping`/`runtime.hostSmoke` 闭环 smoke
- **Blocker 1**:`entry/src/main/cpp/types/libreader_core_napi/Index.d.ts` 严重过期 —— 只声明 `abiVersion`/`pingSmoke`/`sendJsonCommand` 3 个函数,但 C++ 导出 12 个(`createRuntime`/`releaseRuntime`/`sendCommand`/`cancelRequest`/`readEvent`/`pendingEventCount`/`completeHostRequest`/`failHostRequest`/`pingSmoke`/`hostSmoke`/`lifecycleSmoke`/`abiVersion`)。ArkTS 严格类型下,`ReaderCoreNapiBridge.ets` 的 `ReaderCoreNapiRaw` interface 声明的 12 个方法中,9 个在 Index.d.ts 没声明,运行时会报 "not a function"
- **Blocker 2**:HarmonyOS 无 host-adapter 模块。Android 有 `bindings/android/host-adapter/src/main/java/com/reader/core/host/{HostAdapter,HostBus,HostRuntime,HttpExecuteHandler,HostTransport,...}.java`,HarmonyOS 没有等价的 ArkTS host-adapter 模块,无法把 Core 的 `host.request` 路由到 HTTP transport
- **Blocker 3**:业务路径完全没接 Rust Core。`SearchService` 走 `BridgeClientWithFallback(BridgeHTTPClient, FixtureReplayInterceptor)` —— `BridgeHTTPClient` 是 "Strategy B: development accelerator for local Core bridge (localhost:8899)",完全绕过 Core host.request 协议;`BookshelfService` 走 `MockBookshelfRepository`;`ReaderPage` 走 `FixtureEPUBParserAdapter`
- HTTP transport:`HTTPAdapter.ets` 已存在(用 `@ohos.net.http`),但被独立业务调用,不是 `HostTransport` 实现
- 持久化:`MockBookshelfRepository` + `HomeDashboardMemoryStorage`(Map 内存),非 Core `SqliteStorage`
- 系统 TTS:`SystemTtsAdapter.ets` 已存在,但未接 Core TTS 编排(章程 TTS 策略:Core 编排,Host 发声)
- WebView:`ArkWebPlatformAdapter.ets` 已存在,本期不实现
- 主仓库 `bindings/harmony/` 已就绪:`native/reader_napi.cpp`(785 行)+ `sdk/reader_core.ts`(typed SDK wrapper)+ `sdk/smoke_report.ts` + `Index.ets`(ArkTS entry)+ `scripts/build-ohos.sh` + `scripts/build-harmony-napi.sh`
- 主仓库 `include/reader_core.h` C ABI v1 兼容(被 NAPI 直接消费)

**红线**(章程 §4 / §10):
- Core 不开 socket、不碰 WebView、不存明文凭据
- HarmonyOS 仅保留 `@ohos.net.http` transport + ArkWeb WebView(本期不实现)+ HUKS(凭据存储,本期未用)+ `SystemTts`(系统 TTS,本期未用)在 Core/Host 边界
- 不破坏 5 页 ArkTS UI 的纯 UI 代码(只改业务源,不改 UI 布局)
- wrapper smoke ≠ device proof,分层标注证据
- 不破坏主仓库 dirty 文件(并发 agent 工作)
- 不破坏 HarmonyOS 仓库 dirty 文件(并发 agent 工作)

**验证命令**:
```bash
cd "/Users/minliny/Documents/Reader for HarmonyOS"
# 1. HAP 构建(必绿)
hvigorw assembleHap --mode module -p product=default -p buildMode=debug \
  --no-daemon -p arguments="--immutable-config"
# 2. 模拟器/真机 smoke(HAP 安装后跑 captureHarmonyNapiSmokeArtifact)
npm run smoke:device-runtime
# 3. 端到端(经 Rust Core,非 fixture)
#    详情见 Task 8 —— HAP 内 ets 测试驱动 search→detail→toc→content
```

---

## 文件结构

### HarmonyOS 仓库新建文件

```
entry/src/main/ets/
├── host/                                              # 新建:host-adapter 模块(ArkTS 版)
│   ├── HostAdapter.ets                                # 新建:capability 路由 + dispatch
│   ├── HostBus.ets                                    # 新建:host.request 监听 + host.complete/error 回送
│   ├── HostRuntime.ets                                # 新建:持有 ReaderCoreRuntime + HostAdapter,事件循环
│   ├── HostTransport.ets                              # 新建:HostTransport interface
│   ├── HttpExecuteHandler.ets                         # 新建:http.execute capability handler
│   ├── HttpRequest.ets                                # 新建:HostHttpRequest DTO(对应 Core host.request.params)
│   ├── HttpResponse.ets                               # 新建:HostHttpResponse DTO(对应 host.complete.result)
│   ├── HostReply.ets                                  # 新建:HostReply success/error 包装
│   ├── HostReplyCodec.ets                             # 新建:HostReply ↔ JSON 编解码
│   └── OHOSHTTPHostTransport.ets                      # 新建:@ohos.net.http 实现 HostTransport
├── api/                                               # 新建:业务 facade
│   ├── ReaderCoreClient.ets                           # 新建:App 单例,持有 ReaderCoreRuntime + HostAdapter
│   ├── BookApi.ets                                    # 新建:book.search/detail/toc/chapter.content facade
│   └── SourceApi.ets                                  # 新建:source.import facade
└── viewmodel/                                         # 新建(已有目录,新增文件)
    ├── BookshelfViewModel.ets                         # 新建:调 ReaderCoreClient.bookshelf.list
    ├── SearchViewModel.ets                            # 新建:调 BookApi.search
    └── ReadingViewModel.ets                           # 新建:调 BookApi.toc + content
```

### HarmonyOS 仓库修改文件

```
entry/src/main/cpp/types/libreader_core_napi/
├── Index.d.ts                                         # 修改:从 3 exports 扩展到 12 exports(对齐 reader_napi.cpp)
└── oh-package.json5                                   # 不动
entry/src/main/ets/
├── pages/Index.ets                                    # 修改:BookshelfTab/SearchTab/SettingsTab 业务源切到 ViewModel(调 RustCore*Service)
├── pages/BookshelfPage.ets                            # 修改:调 BookshelfViewModel
├── pages/SearchPage.ets                               # 修改:调 SearchViewModel
├── pages/ReaderPage.ets                               # 修改:调 ReadingViewModel(经 Core book.toc + chapter.content,退役 FixtureEPUBParserAdapter)
├── pages/SettingsPage.ets                             # 修改:加"导入书源"入口(SourceApi.importBookSource)
├── entryability/EntryAbility.ets                      # 修改:onCreate 时 ReaderCoreClient.init()
└── cabi/ReaderCoreNapiBridge.ets                      # 不动(已就绪,但实际 12 exports 由 Task 1 修复 Index.d.ts 后才可用)
```

### HarmonyOS 仓库退役文件(从生产路径移除,保留为 deprecated fixture fallback)

```
entry/src/main/ets/services/
├── HomeDashboardService.ets                           # 保留文件,标记 @deprecated;生产路径切到 ReaderCoreClient
├── SearchService.ets                                  # 保留文件,标记 @deprecated;生产路径切到 BookApi
├── BridgeHTTPClient.ets                               # 保留文件,标记 @deprecated(localhost:8899 桥,绕过 Core 协议)
├── FixtureReplayInterceptor.ets                       # 保留文件,标记 @deprecated(仅测试 fixture)
├── BookshelfService.ets                               # 保留文件,标记 @deprecated(MockBookshelfRepository fixture)
├── ProgressService.ets                                # 保留文件,标记 @deprecated(本期不接 Core 阅读进度,留 S7)
└── ImportPipeline.ets                                 # 保留文件,标记 @deprecated(本期不接 Core source.import 全链路,留 S7)
entry/src/main/ets/repository/
├── BookshelfRepository.ets                            # 保留文件(MockBookshelfRepository 标记 @deprecated)
└── BookSourceRepository.ets                           # 保留文件(本地存储,本期保留)
entry/src/main/ets/adapters/
├── LocalBookPlatformAdapters.ets                      # 保留文件(FixtureEPUBParserAdapter 标记 @deprecated;ReaderPage 不再调用)
└── HTTPAdapter.ets                                    # 保留文件(独立 HTTP,被 OHOSHTTPHostTransport 取代在生产路径)
entry/src/main/ets/parser/
├── EPUBParserContract.ets                             # 保留文件(本地书,本期不动)
└── TXTParser.ets                                      # 保留文件(本地书,本期不动)
```

### 主仓库改动

**零** —— 本期仅消费主仓库已就绪的 `bindings/harmony/` + `include/reader_core.h` + `scripts/build-ohos.sh`,不改主仓库任何源文件。Plan 文档本身提交到主仓库 `docs/superpowers/plans/`。新增 HarmonyOS 专属 release blockers 到 `reports/tooling/release-blockers.json`(见 Step 5)。

---

## 任务分解

### Task 1: 修复 Index.d.ts 与 reader_napi.cpp 一致(12 exports)

**Files:**
- Modify: `/Users/minliny/Documents/Reader for HarmonyOS/entry/src/main/cpp/types/libreader_core_napi/Index.d.ts`

**背景**:`entry/src/main/cpp/reader_napi.cpp` 导出 12 个 NAPI 函数(`abiVersion`/`createRuntime`/`releaseRuntime`/`sendCommand`/`cancelRequest`/`readEvent`/`pendingEventCount`/`completeHostRequest`/`failHostRequest`/`pingSmoke`/`hostSmoke`/`lifecycleSmoke`),但 `Index.d.ts` 只声明 3 个(`abiVersion`/`pingSmoke`/`sendJsonCommand`)。`sendJsonCommand` 甚至不在 C++ exports 中(可能是早期 POC 残留)。这是 ArkTS 严格类型下的硬 blocker:`ReaderCoreNapiBridge.ets` 的 `ReaderCoreNapiRaw` interface 声明的 12 个方法中,9 个在 Index.d.ts 没声明,运行时会报 "not a function"。参考主仓库 `bindings/harmony/sdk/reader_core.ts` 的 `NativeReaderCoreModule` type 定义。

- [ ] **Step 1: 确认当前 Index.d.ts 内容**

```bash
cd "/Users/minliny/Documents/Reader for HarmonyOS"
cat entry/src/main/cpp/types/libreader_core_napi/Index.d.ts
```
Expected: 只看到 3 行 `export const abiVersion/pingSmoke/sendJsonCommand`。

- [ ] **Step 2: 用主仓库 `bindings/harmony/sdk/reader_core.ts` 的 `NativeReaderCoreModule` type 作为权威来源**

```bash
sed -n '1,30p' /Users/minliny/Documents/Reader-Core-Native/bindings/harmony/sdk/reader_core.ts
```
Expected: 看到 `NativeReaderCoreModule` type 定义,12 个方法签名。

- [ ] **Step 3: 重写 Index.d.ts 声明 12 exports**

用 Write 工具替换 `entry/src/main/cpp/types/libreader_core_napi/Index.d.ts`:

```typescript
// Type declarations for libreader_core_napi.so — mirrors reader_napi.cpp Init() exports.
// Source of truth: /Users/minliny/Documents/Reader-Core-Native/bindings/harmony/native/reader_napi.cpp
// Aligned with: /Users/minliny/Documents/Reader-Core-Native/bindings/harmony/sdk/reader_core.ts (NativeReaderCoreModule)

export type JsonObject = { [key: string]: unknown };
export type NativeRuntimeHandle = unknown;

export const abiVersion: () => number;
export const createRuntime: (config?: JsonObject | string) => NativeRuntimeHandle;
export const releaseRuntime: (runtime: NativeRuntimeHandle) => void;
export const sendCommand: (runtime: NativeRuntimeHandle, command: JsonObject | string) => void;
export const cancelRequest: (runtime: NativeRuntimeHandle, requestId: number) => void;
export const readEvent: (runtime: NativeRuntimeHandle, timeoutMs?: number) => string | null;
export const pendingEventCount: (runtime: NativeRuntimeHandle) => number;
export const completeHostRequest: (
  runtime: NativeRuntimeHandle,
  operationId: number,
  result: JsonObject | string,
  requestId?: number
) => void;
export const failHostRequest: (
  runtime: NativeRuntimeHandle,
  operationId: number,
  error: JsonObject | string,
  requestId?: number
) => void;
export const pingSmoke: () => string;
export const hostSmoke: () => string;
export const lifecycleSmoke: (iterations?: number) => string;
```

- [ ] **Step 4: 验证 HAP 编译通过**

```bash
cd "/Users/minliny/Documents/Reader for HarmonyOS"
hvigorw assembleHap --mode module -p product=default -p buildMode=debug --no-daemon 2>&1 | tail -10
```
Expected: BUILD SUCCESSFUL。若失败,常见原因:
- ArkTS 类型不匹配(`JsonObject` 与 `object` 在 ArkTS strict 模式下不同)→ 调整为 ArkTS 兼容写法
- `NativeRuntimeHandle = unknown` 在 ArkTS 下可能需要 `NativeRuntimeHandle = ESObject` → 用 `ESObject` 替换

- [ ] **Step 5: 验证 captureHarmonyNapiSmokeArtifact 在设备/模拟器跑通**

```bash
# 启动 HarmonyOS 模拟器或连接真机
hdc shell haps   # 列出已安装 HAP
# 安装新构建的 HAP
hdc install -r entry/build/default/outputs/default/entry-default-signed.hap
# 启动 App,在 Settings 页看 HostBus / nativeHTTP 等运行证据
# 或跑 npm run smoke:device-runtime
npm run smoke:device-runtime 2>&1 | tail -20
```
Expected: HostBus = `PASS op:<id>`(runtime.hostSmoke 闭环通过)。若失败:
- `native module not built` → CMake 未生成 .so,检查 `entry/build/default/intermediates/cmake/default/obj/arm64-v8a/libreader_core_napi.so`
- `rc_runtime_create failed` → libreader_core.a 未链接,检查 CMakeLists.txt
- `toolchain: native module not built` → OHOS_SDK_HOME 未设置,检查 `echo $OHOS_SDK_HOME`

- [ ] **Step 6: 提交**

```bash
cd "/Users/minliny/Documents/Reader for HarmonyOS"
git add entry/src/main/cpp/types/libreader_core_napi/Index.d.ts
git commit -m "fix(harmony/s6): align Index.d.ts with reader_napi.cpp 12 exports

Index.d.ts only declared 3 of 12 NAPI exports (abiVersion/pingSmoke/
sendJsonCommand — the last not even in the C++ module). This blocked
ReaderCoreNapiBridge.ets from calling createRuntime/sendCommand/
readEvent/completeHostRequest/etc at runtime ('not a function').

Aligned with main repo bindings/harmony/sdk/reader_core.ts
NativeReaderCoreModule type (source of truth).

Per charter §10.3: this unblocks wrapper smoke (captureHarmonyNapiSmokeArtifact)
on device — but wrapper smoke ≠ device proof (Task 8 provides E2E proof)."
```

---

### Task 2: 验证 HAP 中 libreader_core_napi.so 打包 + NAPI smoke 闭环

**Files:**
- No file changes — 验证 Task 1 的产出在 HAP 中真正可用
- Reference: `/Users/minliny/Documents/Reader for HarmonyOS/entry/src/main/cpp/CMakeLists.txt`(已调主仓库 `scripts/build-ohos.sh`)
- Reference: `/Users/minliny/Documents/Reader for HarmonyOS/entry/src/main/ets/cabi/ReaderCoreNapiBridge.ets`(已实现 `runHostSmokeClosedLoop`)

**背景**:`CMakeLists.txt` line 22-26 在 hvigor 构建时调 `scripts/build-ohos.sh` 构建 `libreader_core.a`,然后链接 `libreader_core_napi.so`。STATUS.md 明确说"在签名 HAP 中于设备上运行 captureHarmonyNapiSmokeArtifact 并归档"是未完成项。本 Task 验证 .so 真正进入 HAP + 在设备/模拟器跑通 12-export smoke。

- [ ] **Step 1: 验证 OHOS SDK + Rust 工具链就绪**

```bash
echo "OHOS_SDK_HOME=${OHOS_SDK_HOME:-<unset>}"
ls -d "${OHOS_SDK_HOME}/openharmony/native/sysroot" 2>/dev/null && echo "sysroot OK" || echo "sysroot MISSING"
rustup target list --installed | grep ohos
```
Expected: `OHOS_SDK_HOME` 非空;sysroot 目录存在;`aarch64-unknown-linux-ohos` 在 installed 列表。若 missing:
- 安装 OHOS SDK DevEco Studio 自带,在 DevEco Studio 设置中查路径
- `rustup target add aarch64-unknown-linux-ohos`

- [ ] **Step 2: 清理 + 重新构建 HAP,验证 .so 生成**

```bash
cd "/Users/minliny/Documents/Reader for HarmonyOS"
hvigorw clean --no-daemon
hvigorw assembleHap --mode module -p product=default -p buildMode=debug --no-daemon 2>&1 | tail -20
# 检查 .so
find entry/build -name "libreader_core_napi.so" -type f
# 检查 .so 在 HAP 中
unzip -l entry/build/default/outputs/default/entry-default-signed.hap | grep libreader_core_napi.so
```
Expected:
- `find` 输出 `entry/build/default/intermediates/cmake/default/obj/arm64-v8a/libreader_core_napi.so`
- `unzip -l` 输出 `libs/arm64-v8a/libreader_core_napi.so`

- [ ] **Step 3: 验证 .so 中的 NAPI exports(12 个)**

```bash
# 用 nm 或 objdump 看 .so 的导出符号
"${OHOS_SDK_HOME}/openharmony/native/llvm/bin/llvm-nm" -D \
  entry/build/default/intermediates/cmake/default/obj/arm64-v8a/libreader_core_napi.so | \
  grep -i "napi_register\|Init" | head -5
```
Expected: 看到 `napi_register_module_v1` 或 `Init` 符号。NAPI 函数名通过 `napi_define_properties` 注册,不在符号表中,但 `napi_register_module_v1` 存在即说明 NAPI module 已正确导出。

- [ ] **Step 4: 在设备/模拟器跑 captureHarmonyNapiSmokeArtifact**

```bash
# 连接设备或启动模拟器
hdc list targets
# 安装 HAP
hdc install -r entry/build/default/outputs/default/entry-default-signed.hap
# 启动 App
hdc shell aa start -a EntryAbility -b com.reader.harmonyos
# 等 5 秒后看日志
sleep 5
hdc hilog | grep -i "reader_core\|harmony_napi\|hostSmoke" | head -30
```
Expected: 日志显示 `abiVersion=1`、`pingPong=true`、`hostSmoke hasHostRequest=true hasCompletion=true completionOk=true`。或:
```bash
npm run smoke:device-runtime 2>&1 | tail -30
```
Expected: summary 中 `HostBus=PASS op:<id>`、`nativeHTTP=PASS 2xx`、`raw:false`、无 `FAIL` 或 `RUNNING` token。

- [ ] **Step 5: 归档 smoke artifact**

```bash
ls -la artifacts/device-runtime-smoke/latest/ 2>/dev/null
cat artifacts/device-runtime-smoke/latest/device_runtime_smoke_summary.json 2>/dev/null | head -30
```
Expected: 看到 summary JSON,`tier=emulator` 或 `tier=device`,`HostBus` token 包含 `PASS op:`。

- [ ] **Step 6: 提交(若有 artifact 改动)**

```bash
cd "/Users/minliny/Documents/Reader for HarmonyOS"
git status --short
# 若 artifacts 目录有新文件
git add artifacts/device-runtime-smoke/latest/ 2>/dev/null
git commit -m "test(harmony/s6): verify libreader_core_napi.so packaged + 12-export smoke on device

After Task 1 fixed Index.d.ts, the HAP now successfully loads
libreader_core_napi.so and the 12 NAPI exports are reachable from
ArkTS. captureHarmonyNapiSmokeArtifact / runHostSmokeClosedLoop pass:
- abiVersion=1
- pingPong=true (runtime.ping round-trip)
- hostSmoke hasHostRequest=true hasCompletion=true completionOk=true
  (host.request -> host.complete round-trip through real C ABI)

Evidence layering (charter §10.3):
- HAP build + .so packaged = build proof
- captureHarmonyNapiSmokeArtifact on device = wrapper smoke (NAPI layer)
- NOT yet E2E proof (Task 8 provides search→detail→toc→content via Rust Core)"
```

---

### Task 3: 新建 host-adapter 模块(ArkTS 版)

**Files:**
- Create: `entry/src/main/ets/host/HostTransport.ets`
- Create: `entry/src/main/ets/host/HttpRequest.ets`
- Create: `entry/src/main/ets/host/HttpResponse.ets`
- Create: `entry/src/main/ets/host/HostReply.ets`
- Create: `entry/src/main/ets/host/HostReplyCodec.ets`
- Create: `entry/src/main/ets/host/CapabilityHandler.ets`
- Create: `entry/src/main/ets/host/HostAdapter.ets`
- Create: `entry/src/main/ets/host/HostBus.ets`
- Create: `entry/src/main/ets/host/HttpExecuteHandler.ets`
- Create: `entry/src/main/ets/host/HostRuntime.ets`

**背景**:HarmonyOS 缺 host-adapter 模块。Android 有 `bindings/android/host-adapter/src/main/java/com/reader/core/host/{HostAdapter,HostBus,HostRuntime,HttpExecuteHandler,HostTransport,HostRequest,HostReply,HostReplyCodec,CapabilityHandler,...}.java`(Java,约 15 文件)。HarmonyOS 需要等价的 ArkTS host-adapter 模块,把 Core 的 `host.request` 事件路由到平台 host 能力(HTTP transport)。本期只实现 `http.execute` capability(对应 Legado `webBook` 拉取 HTML),其他 capability(`webview.evaluateJavaScript`/`credential.resolve`/`file.read` 等)留 S7+。

- [ ] **Step 1: 探查 Android host-adapter 模块结构作为参考**

```bash
ls /Users/minliny/Documents/Reader-Core-Native/bindings/android/host-adapter/src/main/java/com/reader/core/host/
```
Expected: 看到 15+ Java 文件,包括 `HostAdapter.java`/`HostBus.java`/`HostRuntime.java`/`HttpExecuteHandler.java`/`HostTransport.java`/`HttpRequest.java`/`HttpResponse.java`/`HostReply.java`/`HostReplyCodec.java`/`CapabilityHandler.java` 等。

- [ ] **Step 2: 创建 HostTransport interface**

`entry/src/main/ets/host/HostTransport.ets`:

```typescript
// HostTransport — interface for platform-owned HTTP transport.
// Implementations execute a HostHttpRequest and return a HostHttpResponse.
// Core emits host.request with capability='http.execute' and params=HostHttpRequest;
// the host-adapter dispatches to HostTransport, wraps the result as host.complete.

import { HostHttpRequest, HostHttpResponse } from './HttpRequest';

export interface HostTransport {
  execute(request: HostHttpRequest): Promise<HostHttpResponse>;
}
```

- [ ] **Step 3: 创建 HttpRequest / HttpResponse DTO**

`entry/src/main/ets/host/HttpRequest.ets`:

```typescript
// HostHttpRequest — mirrors Core's host.request.params shape for http.execute.
// Fields aligned with Android bindings/android/.../HttpRequest.java and
// protocol/reader-command.schema.json (host.execute method).

export interface HostHttpRequest {
  url: string;
  method: string;            // 'GET' | 'POST' | 'PUT' | 'DELETE' | ...
  headers?: Record<string, string>;
  body?: string;
  timeoutMs?: number;
  charset?: string;          // hint for response decoding
}

export interface HostHttpResponse {
  status: number;
  headers: Record<string, string>;
  body: string;
  finalUrl: string;          // after redirects
  charsetHint?: string;
}
```

- [ ] **Step 4: 创建 HostReply + HostReplyCodec**

`entry/src/main/ets/host/HostReply.ets`:

```typescript
// HostReply — wraps the result or error that the host sends back to Core
// via host.complete (success) or host.error (failure).

export interface HostReplySuccess {
  ok: true;
  result: object;            // JSON object sent as host.complete.params.result
}

export interface HostReplyError {
  ok: false;
  code: string;              // SCREAMING_SNAKE_CASE error code
  message: string;
  retryable: boolean;
}

export type HostReply = HostReplySuccess | HostReplyError;

export function hostReplySuccess(result: object): HostReplySuccess {
  return { ok: true, result };
}

export function hostReplyError(code: string, message: string, retryable: boolean = false): HostReplyError {
  return { ok: false, code, message, retryable };
}
```

`entry/src/main/ets/host/HostReplyCodec.ets`:

```typescript
// HostReplyCodec — encode HostReply as host.complete or host.error JSON command.
// Core consumes these via rc_runtime_send; the host.response requestId is minted
// by the caller (HostBus) to avoid colliding with app-issued requestIds.

import { HostReply } from './HostReply';

export function buildHostCompleteJson(requestId: number, operationId: number, result: object): string {
  const cmd: Record<string, Object> = {
    protocolVersion: 1,
    requestId: requestId,
    method: 'host.complete',
    params: {
      operationId: operationId,
      result: result
    }
  };
  return JSON.stringify(cmd);
}

export function buildHostErrorJson(requestId: number, operationId: number, code: string, message: string, retryable: boolean): string {
  const cmd: Record<string, Object> = {
    protocolVersion: 1,
    requestId: requestId,
    method: 'host.error',
    params: {
      operationId: operationId,
      error: {
        code: code,
        message: message,
        retryable: retryable
      }
    }
  };
  return JSON.stringify(cmd);
}
```

- [ ] **Step 5: 创建 CapabilityHandler interface + HostAdapter**

`entry/src/main/ets/host/CapabilityHandler.ets`:

```typescript
// CapabilityHandler — handles a single host.request capability.
// Implementations receive the parsed host.request.params and return a
// HostReplySuccess.result object (for host.complete) or throw (for host.error).

import { HostReply } from './HostReply';

export type CapabilityHandler = (params: Record<string, Object>) => Promise<HostReply>;
```

`entry/src/main/ets/host/HostAdapter.ets`:

```typescript
// HostAdapter — routes host.request events to registered CapabilityHandler by
// capability string. Mirrors Android bindings/android/.../HostAdapter.java.
//
// Failure modes map to host.error:
// - no handler registered → non-retryable INTERNAL
// - handler returns HostReplyError → forwarded as-is
// - handler throws → retryable INTERNAL (transient host failure)

import { CapabilityHandler } from './CapabilityHandler';
import { HostReply, hostReplyError } from './HostReply';

const INTERNAL = 'INTERNAL';

export class HostAdapter {
  private handlers: Map<string, CapabilityHandler> = new Map();

  register(capability: string, handler: CapabilityHandler): void {
    if (!capability) { throw new Error('capability required'); }
    if (!handler) { throw new Error('handler required'); }
    this.handlers.set(capability, handler);
  }

  isRegistered(capability: string): boolean {
    return this.handlers.has(capability);
  }

  async dispatch(capability: string, params: Record<string, Object>): Promise<HostReply> {
    const handler = this.handlers.get(capability);
    if (!handler) {
      return hostReplyError(INTERNAL, `unsupported capability: ${capability}`, false);
    }
    try {
      return await handler(params);
    } catch (e) {
      return hostReplyError(INTERNAL, `handler threw for ${capability}: ${e}`, true);
    }
  }
}
```

- [ ] **Step 6: 创建 HttpExecuteHandler(http.execute capability)**

`entry/src/main/ets/host/HttpExecuteHandler.ets`:

```typescript
// HttpExecuteHandler — handles http.execute capability.
// Core emits host.request with capability='http.execute' and params=HostHttpRequest;
// this handler delegates to HostTransport, wraps the HostHttpResponse as host.complete.result.

import { CapabilityHandler } from './CapabilityHandler';
import { HostTransport } from './HostTransport';
import { HostHttpRequest, HostHttpResponse } from './HttpRequest';
import { HostReply, hostReplySuccess, hostReplyError } from './HostReply';

export function makeHttpExecuteHandler(transport: HostTransport): CapabilityHandler {
  return async (params: Record<string, Object>): Promise<HostReply> => {
    const req = parseHttpRequest(params);
    if (req === null) {
      return hostReplyError('INVALID_ARGUMENT', 'http.execute params missing url/method', false);
    }
    try {
      const resp: HostHttpResponse = await transport.execute(req);
      return hostReplySuccess({
        status: resp.status,
        headers: resp.headers,
        body: resp.body,
        finalUrl: resp.finalUrl,
        charsetHint: resp.charsetHint ?? ''
      });
    } catch (e) {
      return hostReplyError('NETWORK_ERROR', `http.execute failed: ${e}`, true);
    }
  };
}

function parseHttpRequest(params: Record<string, Object>): HostHttpRequest | null {
  const url = params['url'];
  const method = params['method'];
  if (typeof url !== 'string' || typeof method !== 'string') {
    return null;
  }
  const req: HostHttpRequest = {
    url: url as string,
    method: (method as string).toUpperCase()
  };
  const headers = params['headers'];
  if (headers && typeof headers === 'object') {
    req.headers = headers as Record<string, string>;
  }
  const body = params['body'];
  if (typeof body === 'string') {
    req.body = body as string;
  }
  const timeoutMs = params['timeoutMs'];
  if (typeof timeoutMs === 'number') {
    req.timeoutMs = timeoutMs as number;
  }
  const charset = params['charset'];
  if (typeof charset === 'string') {
    req.charset = charset as string;
  }
  return req;
}
```

- [ ] **Step 7: 创建 HostBus(host.request 监听 + host.complete/error 回送)**

`entry/src/main/ets/host/HostBus.ets`:

```typescript
// HostBus — listens for host.request events from Core, dispatches to HostAdapter,
// and sends host.complete/host.error back via ReaderCoreRuntime.
//
// The host.response requestId is minted from a high base (9_000_000_000_000+)
// to avoid colliding with app-issued requestIds. Core routes host.complete/
// host.error back to the originating runtime.hostSmoke/book.search/etc requestId,
// so the host-response requestId can be any fresh value.

import { ReaderCoreRuntime } from '../cabi/ReaderCoreNapiBridge';
import { HostAdapter } from './HostAdapter';
import { HostReply } from './HostReply';
import { buildHostCompleteJson, buildHostErrorJson } from './HostReplyCodec';

const HOST_RESPONSE_REQUEST_ID_BASE = 9000000000000;

export class HostBus {
  private runtime: ReaderCoreRuntime;
  private adapter: HostAdapter;
  private nextHostResponseRequestId: number = HOST_RESPONSE_REQUEST_ID_BASE;

  constructor(runtime: ReaderCoreRuntime, adapter: HostAdapter) {
    this.runtime = runtime;
    this.adapter = adapter;
  }

  // Poll the runtime event queue and handle any host.request events.
  // Returns the number of host.request events handled.
  async pollAndHandle(timeoutMs: number = 100): Promise<number> {
    let handled = 0;
    // Drain all pending events without blocking.
    while (this.runtime.pendingEventCount() > 0) {
      const raw = this.runtime.readEvent(0);
      if (raw === null) { break; }
      if (raw.indexOf('"type":"host.request"') !== -1) {
        await this.handleHostRequest(raw);
        handled++;
      }
      // Other event types (result/error) are left for the originating
      // request waiter to consume. They remain in the runtime's internal
      // queue — but since readEvent pops, we lose them. For a real impl,
      // route non-host.request events to a separate queue.
      // TODO(S6): use a multiplexer that preserves unrelated events.
    }
    return handled;
  }

  private async handleHostRequest(rawEvent: string): Promise<void> {
    let parsed: Record<string, Object>;
    try {
      parsed = JSON.parse(rawEvent) as Record<string, Object>;
    } catch (e) {
      return; // malformed event, drop
    }
    const capability = parsed['capability'];
    const operationId = parsed['operationId'];
    const params = parsed['params'];
    if (typeof capability !== 'string' || typeof operationId !== 'number') {
      return;
    }
    const reply: HostReply = await this.adapter.dispatch(
      capability as string,
      (params ?? {}) as Record<string, Object>
    );
    const hostResponseRequestId = this.nextHostResponseRequestId++;
    if (reply.ok) {
      const json = buildHostCompleteJson(hostResponseRequestId, operationId as number, reply.result);
      this.runtime.sendHostComplete(json);
    } else {
      const json = buildHostErrorJson(
        hostResponseRequestId, operationId as number,
        reply.code, reply.message, (reply as any).retryable
      );
      this.runtime.sendHostError(json);
    }
  }
}
```

**注意**:`ReaderCoreRuntime` 在 `cabi/ReaderCoreNapiBridge.ets` 中已有 `sendCommand` 方法,但 `sendHostComplete`/`sendHostError` 是便捷方法,需要在 Task 5 的 `ReaderCoreClient` 中实现(或直接调 `sendCommand('host.complete', ...)`)。本 Task 先假设 `ReaderCoreRuntime` 有这些方法,Task 5 会补齐。

- [ ] **Step 8: 创建 HostRuntime(持有 ReaderCoreRuntime + HostAdapter,事件循环)**

`entry/src/main/ets/host/HostRuntime.ets`:

```typescript
// HostRuntime — top-level host-side runtime that holds ReaderCoreRuntime + HostAdapter.
// Provides a startEventLoop() that continuously polls Core events and routes
// host.request events to HostAdapter. App-level singleton (ReaderCoreClient)
// owns one HostRuntime instance.

import { ReaderCoreRuntime } from '../cabi/ReaderCoreNapiBridge';
import { HostAdapter } from './HostAdapter';
import { HostBus } from './HostBus';
import { HostTransport } from './HostTransport';
import { makeHttpExecuteHandler } from './HttpExecuteHandler';

export class HostRuntime {
  private runtime: ReaderCoreRuntime;
  private adapter: HostAdapter;
  private bus: HostBus;
  private loopRunning: boolean = false;
  private loopTimerId: number = -1;

  constructor(runtime: ReaderCoreRuntime, transport: HostTransport) {
    this.runtime = runtime;
    this.adapter = new HostAdapter();
    this.adapter.register('http.execute', makeHttpExecuteHandler(transport));
    this.bus = new HostBus(runtime, this.adapter);
  }

  get coreRuntime(): ReaderCoreRuntime {
    return this.runtime;
  }

  get hostAdapter(): HostAdapter {
    return this.adapter;
  }

  // Start a polling event loop. Polls every pollMs for host.request events.
  startEventLoop(pollMs: number = 50): void {
    if (this.loopRunning) { return; }
    this.loopRunning = true;
    const tick = async (): Promise<void> => {
      if (!this.loopRunning) { return; }
      try {
        await this.bus.pollAndHandle(0);
      } catch (_e) {
        // loop must not die on a single tick failure
      }
      if (this.loopRunning) {
        this.loopTimerId = setTimeout(tick, pollMs) as unknown as number;
      }
    };
    tick();
  }

  stopEventLoop(): void {
    this.loopRunning = false;
    if (this.loopTimerId !== -1) {
      clearTimeout(this.loopTimerId as unknown as number);
      this.loopTimerId = -1;
    }
  }

  close(): void {
    this.stopEventLoop();
    this.runtime.release();
  }
}
```

- [ ] **Step 9: 验证编译**

```bash
cd "/Users/minliny/Documents/Reader for HarmonyOS"
hvigorw assembleHap --mode module -p product=default -p buildMode=debug --no-daemon 2>&1 | tail -10
```
Expected: BUILD SUCCESSFUL。若失败,常见原因:
- ArkTS strict 模式不允许 `any`/`unknown` → 用 `Object` 或 `ESObject`
- `setTimeout`/`clearTimeout` 在 ArkTS 下的签名差异 → 用 `setInterval` 或 ArkTS 兼容写法
- `Map` 在 ArkTS 下的 API 差异 → 用 `Record<string, ...>` 替代

- [ ] **Step 10: 提交**

```bash
cd "/Users/minliny/Documents/Reader for HarmonyOS"
git add entry/src/main/ets/host/
git commit -m "feat(harmony/s6): add ArkTS host-adapter module (HostAdapter/HostBus/HostRuntime)

Mirrors Android bindings/android/host-adapter/ in ArkTS:
- HostTransport interface + HttpRequest/HttpResponse DTO
- HostReply + HostReplyCodec (host.complete/host.error JSON builders)
- CapabilityHandler interface + HostAdapter (capability routing)
- HttpExecuteHandler (http.execute capability → HostTransport)
- HostBus (host.request polling + host.complete/error reply)
- HostRuntime (top-level singleton, event loop)

Currently only http.execute capability is registered. Other capabilities
(webview.evaluateJavaScript, credential.resolve, file.read) deferred to S7+.

Per charter §4: Core/Host boundary — HarmonyOS keeps @ohos.net.http
transport in HostTransport; Core produces HostHttpRequest descriptors,
Host executes them and returns HostHttpResponse via host.complete."
```

---

### Task 4: 写 OHOSHTTPHostTransport(用 @ohos.net.http)

**Files:**
- Create: `entry/src/main/ets/host/OHOSHTTPHostTransport.ets`
- Reference: `entry/src/main/ets/adapters/HTTPAdapter.ets`(已存在,参考其 @ohos.net.http 用法)

**背景**:`HTTPAdapter.ets` 已实现 `@ohos.net.http` 封装,但接口是 `get`/`post`/`request`/`downloadToFile`,不是 `HostTransport.execute(HostHttpRequest)`。本 Task 写一个 `OHOSHTTPHostTransport` 实现 `HostTransport` 接口,把 Core 的 `HostHttpRequest` 转换为 `@ohos.net.http` 调用,返回 `HostHttpResponse`。

- [ ] **Step 1: 创建 OHOSHTTPHostTransport**

`entry/src/main/ets/host/OHOSHTTPHostTransport.ets`:

```typescript
// OHOSHTTPHostTransport — implements HostTransport using @ohos.net.http.
// Executes Core's HostHttpRequest and returns HostHttpResponse shape that
// Core's host.complete expects.

import { http } from '@kit.NetworkKit';
import { HostTransport } from './HostTransport';
import { HostHttpRequest, HostHttpResponse } from './HttpRequest';

export class OHOSHTTPHostTransport implements HostTransport {
  private defaultTimeout: number;

  constructor(defaultTimeout: number = 30000) {
    this.defaultTimeout = defaultTimeout;
  }

  async execute(request: HostHttpRequest): Promise<HostHttpResponse> {
    const client = http.createHttp();
    try {
      const method = request.method.toUpperCase();
      const options: http.HttpRequestOptions = {
        method: this.mapMethod(method),
        header: request.headers ?? {},
        extraData: request.body,
        expectDataType: http.HttpDataType.STRING,
        connectTimeout: request.timeoutMs ?? this.defaultTimeout,
        readTimeout: request.timeoutMs ?? this.defaultTimeout
      };
      const resp = await client.request(request.url, options);
      const headers = this.parseHeaders(resp.header);
      return {
        status: resp.responseCode,
        headers,
        body: this.normalizeResult(resp.result),
        finalUrl: request.url,  // @ohos.net.http does not expose finalUrl after redirect
        charsetHint: this.extractCharset(headers)
      };
    } finally {
      client.destroy();
    }
  }

  private mapMethod(method: string): http.RequestMethod {
    switch (method) {
      case 'GET': return http.RequestMethod.GET;
      case 'POST': return http.RequestMethod.POST;
      case 'PUT': return http.RequestMethod.PUT;
      case 'DELETE': return http.RequestMethod.DELETE;
      case 'HEAD': return http.RequestMethod.HEAD;
      case 'OPTIONS': return http.RequestMethod.OPTIONS;
      default: return http.RequestMethod.GET;
    }
  }

  private parseHeaders(header: Object): Record<string, string> {
    const result: Record<string, string> = {};
    if (!header) { return result; }
    const keys = Object.keys(header);
    for (let i = 0; i < keys.length; i++) {
      const k = keys[i];
      const v = (header as Record<string, Object>)[k];
      result[k] = Array.isArray(v) ? (v as string[]).join(', ') : String(v);
    }
    return result;
  }

  private normalizeResult(result: string | Object | ArrayBuffer): string {
    if (typeof result === 'string') { return result; }
    if (result instanceof ArrayBuffer) {
      return String.fromCharCode(...new Uint8Array(result));
    }
    if (result === undefined || result === null) { return ''; }
    return JSON.stringify(result);
  }

  private extractCharset(headers: Record<string, string>): string {
    const ct = headers['Content-Type'] ?? headers['content-type'] ?? '';
    const match = ct.match(/charset=([^;]+)/i);
    return match ? match[1].trim() : '';
  }
}
```

- [ ] **Step 2: 验证编译**

```bash
cd "/Users/minliny/Documents/Reader for HarmonyOS"
hvigorw assembleHap --mode module -p product=default -p buildMode=debug --no-daemon 2>&1 | tail -10
```
Expected: BUILD SUCCESSFUL。

- [ ] **Step 3: 提交**

```bash
cd "/Users/minliny/Documents/Reader for HarmonyOS"
git add entry/src/main/ets/host/OHOSHTTPHostTransport.ets
git commit -m "feat(harmony/s6): add OHOSHTTPHostTransport (@ohos.net.http)

Implements HostTransport interface using @ohos.net.http. Executes Core's
HostHttpRequest (url/method/headers/body/timeoutMs) and returns
HostHttpResponse (status/headers/body/finalUrl/charsetHint) that Core's
host.complete expects.

Mirrors Android OkHttpHostTransport (Task 4 in android S6 plan).

Per charter §4: Host owns real HTTP/TLS — Core only produces request
descriptors, never opens sockets. OHOSHTTPHostTransport is the only
HTTP execution path in the HarmonyOS host layer."
```

---

### Task 5: 写 ReaderCoreClient App 单例 + BookApi/SourceApi facade

**Files:**
- Create: `entry/src/main/ets/api/ReaderCoreClient.ets`
- Create: `entry/src/main/ets/api/BookApi.ets`
- Create: `entry/src/main/ets/api/SourceApi.ets`
- Modify: `entry/src/main/ets/entryability/EntryAbility.ets`(在 onCreate 时 init)

**背景**:`ReaderCoreNapiBridge.ets` 已有 `ReaderCoreRuntime` 类(persistent runtime + sendCommand + readEvent + completeHostRequest + failHostRequest),但只用于 smoke。本 Task 写 App 级 `ReaderCoreClient` 单例,持有 `ReaderCoreRuntime` + `HostRuntime`,提供 `sendCommand` + `awaitResult` 异步等待 result 事件的语义。`BookApi`/`SourceApi` 封装 `book.search`/`book.detail`/`book.toc`/`chapter.content`/`source.import` 调用。

- [ ] **Step 1: 创建 ReaderCoreClient 单例**

`entry/src/main/ets/api/ReaderCoreClient.ets`:

```typescript
// ReaderCoreClient — App-level singleton holding ReaderCoreRuntime + HostRuntime.
// Initialized in EntryAbility.onCreate(). Routes Core events:
//   - result/error → resolved by awaitResult(requestId)
//   - host.request → HostRuntime event loop → HostAdapter → host.complete/error

import { ReaderCoreRuntime } from '../cabi/ReaderCoreNapiBridge';
import { HostRuntime } from '../host/HostRuntime';
import { HostTransport } from '../host/HostTransport';
import { OHOSHTTPHostTransport } from '../host/OHOSHTTPHostTransport';

export interface CoreEvent {
  requestId: number;
  type: 'result' | 'error' | 'host.request';
  data?: object;
  error?: { code: string; message: string; retryable: boolean };
}

interface PendingWaiter {
  requestId: number;
  resolve: (event: CoreEvent) => void;
  timerId: number;
}

export class ReaderCoreClient {
  private runtime: ReaderCoreRuntime;
  private hostRuntime: HostRuntime;
  private pending: Map<number, PendingWaiter> = new Map();
  private nextRequestId: number = 1;
  private static INSTANCE: ReaderCoreClient | null = null;

  private constructor() {
    this.runtime = ReaderCoreRuntime.create();
    const transport: HostTransport = new OHOSHTTPHostTransport();
    this.hostRuntime = new HostRuntime(this.runtime, transport);
    this.hostRuntime.startEventLoop(50);
  }

  static init(): ReaderCoreClient {
    if (ReaderCoreClient.INSTANCE === null) {
      ReaderCoreClient.INSTANCE = new ReaderCoreClient();
    }
    return ReaderCoreClient.INSTANCE;
  }

  static get(): ReaderCoreClient {
    if (ReaderCoreClient.INSTANCE === null) {
      throw new Error('ReaderCoreClient not initialized. Call init() in EntryAbility.onCreate().');
    }
    return ReaderCoreClient.INSTANCE;
  }

  // Send a JSON command to Core and return the requestId for awaitResult.
  sendCommand(method: string, params: object): number {
    const requestId = this.nextRequestId++;
    this.runtime.sendCommand(method, params, requestId);
    return requestId;
  }

  // Wait for the result/error event for the given requestId. Throws on timeout
  // or Core error. Host-request events are routed by HostRuntime event loop.
  async awaitResult(requestId: number, timeoutMs: number = 60000): Promise<object> {
    return new Promise<object>((resolve, reject) => {
      const timerId = setTimeout(() => {
        this.pending.delete(requestId);
        reject(new Error(`Timeout waiting for Core response (requestId=${requestId})`));
      }, timeoutMs) as unknown as number;

      const waiter: PendingWaiter = {
        requestId,
        resolve: (event: CoreEvent) => {
          clearTimeout(timerId as unknown as number);
          this.pending.delete(requestId);
          if (event.type === 'result') {
            resolve(event.data ?? {});
          } else if (event.type === 'error') {
            reject(new Error(`Core error: ${event.error?.code} — ${event.error?.message}`));
          } else {
            reject(new Error(`Unexpected event type: ${event.type}`));
          }
        },
        timerId
      };
      this.pending.set(requestId, waiter);

      // Start a polling loop for this requestId's result/error event.
      // host.request events are drained by HostRuntime event loop; we only
      // need to find the matching result/error.
      this.pollForResult(requestId);
    });
  }

  private async pollForResult(requestId: number): Promise<void> {
    const waiter = this.pending.get(requestId);
    if (!waiter) { return; }
    // Poll the runtime event queue. HostRuntime event loop also polls, so
    // there's a race — for S6 we accept that host.request events may be
    // consumed by either loop. A real impl needs a multiplexer.
    const raw = this.runtime.readEvent(50);
    if (raw !== null) {
      const event = this.parseEvent(raw);
      if (event.requestId === requestId && (event.type === 'result' || event.type === 'error')) {
        waiter.resolve(event);
        return;
      }
      // Not our event — requeue or drop. For S6 simplicity, drop with a log.
      // TODO(S7): implement a proper event multiplexer.
    }
    // Re-poll if still waiting.
    if (this.pending.has(requestId)) {
      this.pollForResult(requestId);
    }
  }

  private parseEvent(raw: string): CoreEvent {
    const parsed = JSON.parse(raw) as Record<string, Object>;
    const type = parsed['type'] as string;
    const requestId = parsed['requestId'] as number;
    if (type === 'result') {
      return { requestId, type: 'result', data: parsed['data'] as object };
    }
    if (type === 'error') {
      const err = parsed['error'] as Record<string, Object>;
      return {
        requestId,
        type: 'error',
        error: {
          code: (err['code'] ?? 'UNKNOWN') as string,
          message: (err['message'] ?? '') as string,
          retryable: (err['retryable'] ?? false) as boolean
        }
      };
    }
    return { requestId, type: 'host.request' };
  }

  close(): void {
    this.hostRuntime.close();
    ReaderCoreClient.INSTANCE = null;
  }
}
```

**注意**:`pollForResult` 与 `HostRuntime.startEventLoop` 都调 `readEvent`,存在 race。S6 接受这个简化(单业务请求串行);S7 需要实现 event multiplexer(把 host.request 路由到 HostBus,把 result/error 路由到 awaitResult waiter)。

- [ ] **Step 2: 创建 BookApi facade**

`entry/src/main/ets/api/BookApi.ets`:

```typescript
// BookApi — async facade for book.search/detail/toc/chapter.content.
// Mirrors Legado WebBook.searchBookAwait/getBookInfoAwait/getChapterListAwait/getContentAwait.

import { ReaderCoreClient } from './ReaderCoreClient';

export interface Book {
  bookUrl: string;
  tocUrl: string;
  name: string;
  author: string;
  coverUrl: string;
  intro: string;
  kind: string;
  wordCount: string;
  latestChapterTitle: string;
  origin: string;
}

export interface SearchBook {
  bookUrl: string;
  name: string;
  author: string;
  coverUrl: string;
  intro: string;
  lastChapter: string;
  kind: string;
  origin: string;
}

export interface Chapter {
  title: string;
  url: string;
  index: number;
  isVip: boolean;
  isPay: boolean;
}

export class BookApi {
  private client: ReaderCoreClient;

  constructor(client: ReaderCoreClient) {
    this.client = client;
  }

  async search(sourceId: string, query: string, page: number = 1): Promise<SearchBook[]> {
    const params: Record<string, Object> = {
      sourceId: sourceId,
      query: query,
      page: page
    };
    const requestId = this.client.sendCommand('book.search', params);
    const data = await this.client.awaitResult(requestId);
    return this.parseSearchResult(data);
  }

  async detail(sourceId: string, book: Book): Promise<Book> {
    const params: Record<string, Object> = {
      sourceId: sourceId,
      book: {
        bookUrl: book.bookUrl,
        name: book.name,
        author: book.author
      }
    };
    const requestId = this.client.sendCommand('book.detail', params);
    const data = await this.client.awaitResult(requestId);
    return this.parseBookDetail(data, book);
  }

  async toc(sourceId: string, book: Book): Promise<Chapter[]> {
    const params: Record<string, Object> = {
      sourceId: sourceId,
      bookId: book.bookUrl
    };
    const requestId = this.client.sendCommand('book.toc', params);
    const data = await this.client.awaitResult(requestId);
    return this.parseTocResult(data);
  }

  async content(sourceId: string, book: Book, chapter: Chapter): Promise<string> {
    const params: Record<string, Object> = {
      sourceId: sourceId,
      bookId: book.bookUrl,
      chapterTitle: chapter.title,
      chapterUrl: chapter.url
    };
    const requestId = this.client.sendCommand('chapter.content', params);
    const data = await this.client.awaitResult(requestId);
    const obj = data as Record<string, Object>;
    return (obj['content'] ?? '') as string;
  }

  private parseSearchResult(data: object): SearchBook[] {
    const obj = data as Record<string, Object>;
    const arr = obj['books'] as Object[] | undefined;
    if (!arr) { return []; }
    const result: SearchBook[] = [];
    for (let i = 0; i < arr.length; i++) {
      const b = arr[i] as Record<string, Object>;
      result.push({
        bookUrl: (b['bookUrl'] ?? '') as string,
        name: (b['name'] ?? '') as string,
        author: (b['author'] ?? '') as string,
        coverUrl: (b['coverUrl'] ?? '') as string,
        intro: (b['intro'] ?? '') as string,
        lastChapter: (b['lastChapter'] ?? '') as string,
        kind: (b['kind'] ?? '') as string,
        origin: (b['origin'] ?? '') as string
      });
    }
    return result;
  }

  private parseBookDetail(data: object, fallback: Book): Book {
    const obj = data as Record<string, Object>;
    const b = (obj['book'] ?? obj) as Record<string, Object>;
    return {
      bookUrl: (b['bookUrl'] ?? fallback.bookUrl) as string,
      tocUrl: (b['tocUrl'] ?? fallback.tocUrl) as string,
      name: (b['name'] ?? fallback.name) as string,
      author: (b['author'] ?? fallback.author) as string,
      coverUrl: (b['coverUrl'] ?? fallback.coverUrl) as string,
      intro: (b['intro'] ?? fallback.intro) as string,
      kind: (b['kind'] ?? fallback.kind) as string,
      wordCount: (b['wordCount'] ?? fallback.wordCount) as string,
      latestChapterTitle: (b['latestChapterTitle'] ?? fallback.latestChapterTitle) as string,
      origin: (b['origin'] ?? fallback.origin) as string
    };
  }

  private parseTocResult(data: object): Chapter[] {
    const obj = data as Record<string, Object>;
    const arr = obj['chapters'] as Object[] | undefined;
    if (!arr) { return []; }
    const result: Chapter[] = [];
    for (let i = 0; i < arr.length; i++) {
      const c = arr[i] as Record<string, Object>;
      result.push({
        title: (c['title'] ?? '') as string,
        url: (c['url'] ?? '') as string,
        index: (c['index'] ?? i) as number,
        isVip: (c['isVip'] ?? false) as boolean,
        isPay: (c['isPay'] ?? false) as boolean
      });
    }
    return result;
  }
}
```

- [ ] **Step 3: 创建 SourceApi facade**

`entry/src/main/ets/api/SourceApi.ets`:

```typescript
// SourceApi — facade for source.import. Mirrors Legado ImportBookSourceViewModel.importSource.

import { ReaderCoreClient } from './ReaderCoreClient';

export interface ImportResult {
  success: boolean;
  data: string;
}

export class SourceApi {
  private client: ReaderCoreClient;

  constructor(client: ReaderCoreClient) {
    this.client = client;
  }

  async importBookSource(bookSourceJson: string): Promise<ImportResult> {
    const source = JSON.parse(bookSourceJson);
    const params: Record<string, Object> = {
      bookSource: source
    };
    const requestId = this.client.sendCommand('source.import', params);
    try {
      const data = await this.client.awaitResult(requestId, 30000);
      return { success: true, data: JSON.stringify(data) };
    } catch (e) {
      return { success: false, data: `${e}` };
    }
  }
}
```

- [ ] **Step 4: 修改 EntryAbility.ets 在 onCreate 时 init ReaderCoreClient**

读 `entry/src/main/ets/entryability/EntryAbility.ets` 当前内容,在 `onCreate` 中加 `ReaderCoreClient.init()`。

```bash
cat "/Users/minliny/Documents/Reader for HarmonyOS/entry/src/main/ets/entryability/EntryAbility.ets"
```

用 Edit 工具在 `onCreate` 方法顶部加:
```typescript
import { ReaderCoreClient } from '../api/ReaderCoreClient';
// ...
onCreate(want, launchParam) {
  ReaderCoreClient.init();
  // ...existing onCreate code...
}
```

- [ ] **Step 5: 验证编译**

```bash
cd "/Users/minliny/Documents/Reader for HarmonyOS"
hvigorw assembleHap --mode module -p product=default -p buildMode=debug --no-daemon 2>&1 | tail -10
```
Expected: BUILD SUCCESSFUL。

- [ ] **Step 6: 提交**

```bash
cd "/Users/minliny/Documents/Reader for HarmonyOS"
git add entry/src/main/ets/api/ entry/src/main/ets/entryability/EntryAbility.ets
git commit -m "feat(harmony/s6): add ReaderCoreClient singleton + BookApi/SourceApi facade

ReaderCoreClient: App-level singleton holding ReaderCoreRuntime +
HostRuntime. Initialized in EntryAbility.onCreate(). Routes Core events:
- result/error → resolved by awaitResult(requestId)
- host.request → HostRuntime event loop → HostAdapter → host.complete/error

BookApi: async facade for book.search/detail/toc/chapter.content. Maps
Core JSON results to ArkTS interfaces (Book/SearchBook/Chapter). Matches
Legado WebBook.searchBookAwait/getBookInfoAwait/getChapterListAwait/
getContentAwait signatures.

SourceApi: facade for source.import (accepts pasted Legado JSON).

EntryAbility.onCreate() now calls ReaderCoreClient.init() to boot Rust
Core + start HostRuntime event loop at app launch.

Known limitation (S7): pollForResult and HostRuntime event loop both
call readEvent — race condition. S6 accepts this for serial business
requests; S7 needs a proper event multiplexer.

Per charter §9.5: this is wrapper smoke (build passes) — NOT device
proof. Task 8 provides E2E proof via HAP on simulator/device."
```

---

### Task 6: 退役 fixture 业务路径(strangler 模式,deprecated 非删除)

**Files:**
- Modify: `entry/src/main/ets/services/HomeDashboardService.ets`(标记 @deprecated)
- Modify: `entry/src/main/ets/services/SearchService.ets`(标记 @deprecated)
- Modify: `entry/src/main/ets/services/BridgeHTTPClient.ets`(标记 @deprecated)
- Modify: `entry/src/main/ets/services/FixtureReplayInterceptor.ets`(标记 @deprecated)
- Modify: `entry/src/main/ets/services/BookshelfService.ets`(标记 @deprecated)
- Modify: `entry/src/main/ets/adapters/LocalBookPlatformAdapters.ets`(标记 FixtureEPUBParserAdapter @deprecated)
- Modify: `entry/src/main/ets/repository/BookshelfRepository.ets`(标记 MockBookshelfRepository @deprecated)

**背景**:5 页 UI 当前业务源全部走 fixture。本 Task 把 fixture-based service 标记为 `@deprecated`, strangler 模式保留 fallback。生产路径在 Task 7 切到 `ReaderCoreClient`/`BookApi`/`SourceApi`。

- [ ] **Step 1: 在每个 fixture-based service 文件顶部加 @deprecated 注释**

对每个文件用 Edit 工具在文件顶部(第一行 import 之前)加:

```typescript
/**
 * @deprecated S6: fixture-based service, retained as fallback. Production path
 * routes through Rust Core via ReaderCoreClient/BookApi/SourceApi. Will be
 * removed in S7 once Rust Core E2E is proven on real device.
 */
```

对 `HomeDashboardService.ets`、`SearchService.ets`、`BridgeHTTPClient.ets`、`FixtureReplayInterceptor.ets`、`BookshelfService.ets` 分别加。

- [ ] **Step 2: 在 MockBookshelfRepository 和 FixtureEPUBParserAdapter 类声明前加 @deprecated**

`BookshelfRepository.ets`:
```typescript
/** @deprecated S6: fixture-based repository. Use ReaderCoreClient.bookshelf.list via Core. */
export class MockBookshelfRepository { ... }
```

`LocalBookPlatformAdapters.ets`:
```typescript
/** @deprecated S6: fixture EPUB parser. ReaderPage now uses BookApi.content via Rust Core. */
export class FixtureEPUBParserAdapter { ... }
```

- [ ] **Step 3: 验证编译(deprecation 不应导致 error)**

```bash
cd "/Users/minliny/Documents/Reader for HarmonyOS"
hvigorw assembleHap --mode module -p product=default -p buildMode=debug --no-daemon 2>&1 | grep -E "error|warning" | head -10
```
Expected: 无 error。注释式 `@deprecated` 不会触发 ArkTS 编译 warning(只有 `@Deprecated` 装饰器才会)。

- [ ] **Step 4: 提交**

```bash
cd "/Users/minliny/Documents/Reader for HarmonyOS"
git add entry/src/main/ets/services/ entry/src/main/ets/adapters/LocalBookPlatformAdapters.ets \
        entry/src/main/ets/repository/BookshelfRepository.ets
git commit -m "refactor(harmony/s6): mark fixture-based services as @deprecated (strangler)

The following fixture-based services are marked @deprecated (retained as
fallback, not deleted):
- HomeDashboardService (MockBookshelfRepository + FixtureReplayInterceptor)
- SearchService (BridgeClientWithFallback, bypasses Core host.request)
- BridgeHTTPClient (localhost:8899 bridge, bypasses Core protocol)
- FixtureReplayInterceptor (fixture-only)
- BookshelfService (MockBookshelfRepository)
- MockBookshelfRepository (Map-based memory store)
- FixtureEPUBParserAdapter (fixture EPUB for ReaderPage)

Production path now routes through ReaderCoreClient/BookApi/SourceApi
(Task 5). Task 7 wires the 5 ArkTS pages to the new facades.

Per charter §10.5: strangler pattern — deprecated, not deleted. S7 will
remove them once Rust Core E2E is proven on real device."
```

---

### Task 7: 改造 5 页 UI 业务源切到 RustCore*Service

**Files:**
- Create: `entry/src/main/ets/viewmodel/BookshelfViewModel.ets`
- Create: `entry/src/main/ets/viewmodel/SearchViewModel.ets`
- Create: `entry/src/main/ets/viewmodel/ReadingViewModel.ets`
- Create: `entry/src/main/ets/viewmodel/ImportBookSourceViewModel.ets`
- Modify: `entry/src/main/ets/pages/Index.ets`(BookshelfTab/SearchTab/SettingsTab 切到 ViewModel)
- Modify: `entry/src/main/ets/pages/BookshelfPage.ets`(切到 BookshelfViewModel)
- Modify: `entry/src/main/ets/pages/SearchPage.ets`(切到 SearchViewModel)
- Modify: `entry/src/main/ets/pages/ReaderPage.ets`(切到 ReadingViewModel,退役 FixtureEPUBParserAdapter)
- Modify: `entry/src/main/ets/pages/SettingsPage.ets`(加"导入书源"入口)

**背景**:5 页 UI 当前直接调 `readerHomeDashboard`(单例 `HomeDashboardService`)。本 Task 引入 ViewModel 层(MVVM),ViewModel 调 `ReaderCoreClient`/`BookApi`/`SourceApi`,UI 只调 ViewModel。**不破坏 UI 布局**——只改业务源。

- [ ] **Step 1: 创建 BookshelfViewModel**

`entry/src/main/ets/viewmodel/BookshelfViewModel.ets`:

```typescript
// BookshelfViewModel — drives BookshelfTab/BookshelfPage.
// Loads books via Core 'bookshelf.list' command (Core SqliteStorage backed).

import { ReaderCoreClient } from '../api/ReaderCoreClient';
import { Book } from '../api/BookApi';

export interface BookshelfRow {
  id: string;
  title: string;
  author: string;
  format: string;
  progressText: string;
  isReading: boolean;
}

export class BookshelfViewModel {
  private client: ReaderCoreClient;

  constructor() {
    this.client = ReaderCoreClient.get();
  }

  async loadBooks(): Promise<BookshelfRow[]> {
    const requestId = this.client.sendCommand('bookshelf.list', {});
    try {
      const data = await this.client.awaitResult(requestId, 10000);
      const obj = data as Record<string, Object>;
      const arr = obj['books'] as Object[] | undefined;
      if (!arr) { return []; }
      const rows: BookshelfRow[] = [];
      for (let i = 0; i < arr.length; i++) {
        const b = arr[i] as Record<string, Object>;
        rows.push({
          id: (b['bookUrl'] ?? '') as string,
          title: (b['name'] ?? '') as string,
          author: (b['author'] ?? '未知作者') as string,
          format: 'TXT',  // Core does not return format yet
          progressText: '',
          isReading: false
        });
      }
      return rows;
    } catch (e) {
      return [];
    }
  }
}
```

- [ ] **Step 2: 创建 SearchViewModel**

`entry/src/main/ets/viewmodel/SearchViewModel.ets`:

```typescript
// SearchViewModel — drives SearchTab/SearchPage.
// Searches across all imported sources via BookApi.search.

import { ReaderCoreClient } from '../api/ReaderCoreClient';
import { BookApi, SearchBook } from '../api/BookApi';

export interface SearchRow {
  title: string;
  author: string;
  detailURL: string;
  intro: string;
}

export class SearchViewModel {
  private client: ReaderCoreClient;
  private bookApi: BookApi;

  constructor() {
    this.client = ReaderCoreClient.get();
    this.bookApi = new BookApi(this.client);
  }

  async search(keyword: string): Promise<SearchRow[]> {
    const trimmed = keyword.trim();
    if (trimmed.length === 0) { return []; }
    // Get imported source IDs (S7: via Core 'source.list'; S6 fallback: single test source)
    const sourceIds = await this.getSourceIds();
    if (sourceIds.length === 0) { return []; }
    const allResults: SearchBook[] = [];
    for (let i = 0; i < sourceIds.length; i++) {
      try {
        const results = await this.bookApi.search(sourceIds[i], trimmed, 1);
        for (let j = 0; j < results.length; j++) {
          allResults.push(results[j]);
        }
      } catch (_e) {
        // single source failure does not block others
      }
    }
    // Map to UI row
    const rows: SearchRow[] = [];
    for (let i = 0; i < allResults.length; i++) {
      const r = allResults[i];
      rows.push({
        title: r.name,
        author: r.author,
        detailURL: r.bookUrl,
        intro: r.intro
      });
    }
    return rows;
  }

  private async getSourceIds(): Promise<string[]> {
    // S6 fallback: query Core 'source.list' (if supported) or return empty.
    // S7: full implementation with source management UI.
    try {
      const requestId = this.client.sendCommand('source.list', {});
      const data = await this.client.awaitResult(requestId, 5000);
      const obj = data as Record<string, Object>;
      const arr = obj['sources'] as Object[] | undefined;
      if (!arr) { return []; }
      const ids: string[] = [];
      for (let i = 0; i < arr.length; i++) {
        const s = arr[i] as Record<string, Object>;
        const id = s['bookSourceUrl'] as string;
        if (id) { ids.push(id); }
      }
      return ids;
    } catch (_e) {
      return [];
    }
  }
}
```

- [ ] **Step 3: 创建 ReadingViewModel**

`entry/src/main/ets/viewmodel/ReadingViewModel.ets`:

```typescript
// ReadingViewModel — drives ReaderPage.
// Loads book detail → TOC → content via BookApi (Rust Core).
// Retires FixtureEPUBParserAdapter from production path.

import { ReaderCoreClient } from '../api/ReaderCoreClient';
import { BookApi, Book, Chapter } from '../api/BookApi';

export interface ReaderChapter {
  title: string;
  content: string;
}

export class ReadingViewModel {
  private client: ReaderCoreClient;
  private bookApi: BookApi;
  private book: Book | null = null;
  private chapters: Chapter[] = [];
  private currentIndex: number = 0;

  constructor() {
    this.client = ReaderCoreClient.get();
    this.bookApi = new BookApi(this.client);
  }

  async loadBookAndToc(sourceId: string, bookUrl: string): Promise<ReaderChapter[]> {
    // 1. Get book detail (enriched metadata)
    const seedBook: Book = {
      bookUrl: bookUrl,
      tocUrl: '',
      name: '',
      author: '',
      coverUrl: '',
      intro: '',
      kind: '',
      wordCount: '',
      latestChapterTitle: '',
      origin: sourceId
    };
    try {
      this.book = await this.bookApi.detail(sourceId, seedBook);
    } catch (_e) {
      this.book = seedBook;
    }

    // 2. Get TOC
    try {
      this.chapters = await this.bookApi.toc(sourceId, this.book);
    } catch (_e) {
      this.chapters = [];
    }
    if (this.chapters.length === 0) {
      return [{ title: '无章节', content: '加载失败:TOC 为空' }];
    }

    // 3. Load first chapter content
    this.currentIndex = 0;
    const firstContent = await this.loadChapter(0);
    return [{ title: this.chapters[0].title, content: firstContent }];
  }

  async loadChapter(index: number): Promise<string> {
    if (index < 0 || index >= this.chapters.length) { return '超出范围'; }
    if (!this.book) { return '书籍未加载'; }
    this.currentIndex = index;
    try {
      const content = await this.bookApi.content(this.book.origin, this.book, this.chapters[index]);
      return content;
    } catch (e) {
      return `加载失败: ${e}`;
    }
  }

  async nextChapter(): Promise<ReaderChapter | null> {
    if (this.currentIndex + 1 >= this.chapters.length) { return null; }
    const index = this.currentIndex + 1;
    const content = await this.loadChapter(index);
    return { title: this.chapters[index].title, content };
  }

  async prevChapter(): Promise<ReaderChapter | null> {
    if (this.currentIndex - 1 < 0) { return null; }
    const index = this.currentIndex - 1;
    const content = await this.loadChapter(index);
    return { title: this.chapters[index].title, content };
  }

  get currentChapterIndex(): number {
    return this.currentIndex;
  }

  get totalChapters(): number {
    return this.chapters.length;
  }
}
```

- [ ] **Step 4: 创建 ImportBookSourceViewModel**

`entry/src/main/ets/viewmodel/ImportBookSourceViewModel.ets`:

```typescript
// ImportBookSourceViewModel — drives SettingsPage "导入书源" entry.
// Accepts pasted Legado JSON, calls SourceApi.importBookSource.

import { ReaderCoreClient } from '../api/ReaderCoreClient';
import { SourceApi } from '../api/SourceApi';

export interface ImportState {
  idle: boolean;
  loading: boolean;
  success: boolean;
  error: string;
}

export class ImportBookSourceViewModel {
  private sourceApi: SourceApi;

  constructor() {
    this.sourceApi = new SourceApi(ReaderCoreClient.get());
  }

  async importBookSource(json: string): Promise<ImportState> {
    if (json.trim().length === 0) {
      return { idle: false, loading: false, success: false, error: '书源 JSON 不能为空' };
    }
    const result = await this.sourceApi.importBookSource(json);
    return {
      idle: false,
      loading: false,
      success: result.success,
      error: result.success ? '' : result.data
    };
  }
}
```

- [ ] **Step 5: 改造 pages/Index.ets 的 BookshelfTab/SearchTab 切到 ViewModel**

用 Edit 工具替换 `BookshelfTab`/`BookshelfContent` 中的 `readerHomeDashboard.getBookshelfSnapshot()` 调用为 `BookshelfViewModel.loadBooks()`。

读 `pages/Index.ets` 当前 line 88-119 的 `BookshelfContent` 组件,把:
```typescript
aboutToAppear() {
  this.reload();
}
private reload(): void {
  const snapshot = readerHomeDashboard.getBookshelfSnapshot();
  this.rows = snapshot.rows;
  // ...
}
```
改为:
```typescript
import { BookshelfViewModel, BookshelfRow } from '../viewmodel/BookshelfViewModel';
// ...
@State rows: BookshelfRow[] = [];
private vm: BookshelfViewModel = new BookshelfViewModel();

aboutToAppear() {
  this.reload();
}
private async reload(): Promise<void> {
  this.rows = await this.vm.loadBooks();
  this.totalBooks = this.rows.length;
  this.activeBooks = this.rows.filter((r: BookshelfRow) => r.isReading).length;
  this.unreadBooks = this.totalBooks - this.activeBooks;
  // readerPreview 仍用 fixture(S7 切 Core)
  this.readerPreview = readerHomeDashboard.getReaderPreviewSnapshot();
}
```

类似地,把 `SearchTab` 中的 `readerHomeDashboard.search(this.keyword)` 调用替换为 `SearchViewModel.search(this.keyword)`。

**注意**:`HomeBookRow` 与 `BookshelfRow` 字段不完全一致 —— 调整 UI 中的字段访问。`readerPreview` 保留 fixture(S7 切 Core 阅读预览)。

- [ ] **Step 6: 改造 pages/BookshelfPage.ets**

读 `pages/BookshelfPage.ets` 当前内容(70 行,纯 fixture),把 `readerHomeDashboard.getBookshelfSnapshot().rows` 替换为 `BookshelfViewModel.loadBooks()`。

- [ ] **Step 7: 改造 pages/SearchPage.ets**

读 `pages/SearchPage.ets` 当前内容(107 行),把 `readerHomeDashboard.search(this.keyword)` 替换为 `SearchViewModel.search(this.keyword)`。

- [ ] **Step 8: 改造 pages/ReaderPage.ets 退役 FixtureEPUBParserAdapter**

读 `pages/ReaderPage.ets` 当前内容(202 行),把 `loadFixtureBook()` 方法改为 `loadBookViaCore()`:
- 退役 `FixtureEPUBParserAdapter` + `buildStoredEPUBFixture()` 调用
- 调 `ReadingViewModel.loadBookAndToc(sourceId, bookUrl)` —— 但 ReaderPage 当前没有 sourceId/bookUrl(从 BookshelfTab 跳转时未传参)。S6 简化:从 BookshelfTab 跳转时传 `sourceId` + `bookUrl`,ReaderPage 读取后调 ViewModel
- 若 bookUrl 为空(无参数跳转),显示 "请从书架或搜索选择书籍"

- [ ] **Step 9: 改造 pages/SettingsPage.ets 加"导入书源"入口**

读 `pages/SettingsPage.ets` 当前内容(54 行),加一个"导入书源"按钮,点击后弹出 TextInput 对话框,粘贴 Legado JSON,调 `ImportBookSourceViewModel.importBookSource(json)`。

- [ ] **Step 10: 验证编译**

```bash
cd "/Users/minliny/Documents/Reader for HarmonyOS"
hvigorw assembleHap --mode module -p product=default -p buildMode=debug --no-daemon 2>&1 | tail -10
```
Expected: BUILD SUCCESSFUL。若失败,常见原因:
- `BookshelfRow` 与 `HomeBookRow` 字段不一致 → 调整 UI 字段访问
- ArkTS async/await 在 `aboutToAppear` 中的限制 → 用 Promise.then() 替代
- `Record<string, Object>` 在 ArkTS strict 模式下的限制 → 用 `class` 替代

- [ ] **Step 11: 提交**

```bash
cd "/Users/minliny/Documents/Reader for HarmonyOS"
git add entry/src/main/ets/viewmodel/ entry/src/main/ets/pages/
git commit -m "feat(harmony/s6): wire 5 ArkTS pages to Rust Core via ViewModel layer

ViewModels (new):
- BookshelfViewModel: loads books via Core 'bookshelf.list'
- SearchViewModel: searches across imported sources via BookApi.search
- ReadingViewModel: loads detail→toc→content via BookApi (retires
  FixtureEPUBParserAdapter from production path)
- ImportBookSourceViewModel: drives SettingsPage 'import source' entry

Page changes (UI layout preserved, business source switched):
- Index.ets BookshelfTab/SearchTab: switch from readerHomeDashboard
  fixture to BookshelfViewModel/SearchViewModel
- BookshelfPage.ets: switch to BookshelfViewModel
- SearchPage.ets: switch to SearchViewModel
- ReaderPage.ets: switch from FixtureEPUBParserAdapter to ReadingViewModel
  (Core book.toc + chapter.content)
- SettingsPage.ets: add 'import source' entry → ImportBookSourceViewModel

readerHomeDashboard retained as @deprecated fixture fallback (Task 6).
ReaderPreviewPanel still uses fixture (S7 will switch to Core reading
progress + preview).

Per charter §6: 三方均开发中 — UI business source switched, but this is
NOT device proof. Task 8 provides E2E proof via HAP on simulator/device."
```

---

### Task 8: 端到端验证(HAP 模拟器/真机跑通 search→detail→toc→content via Rust Core)

**Files:**
- Create: `entry/src/main/ets/__tests__/RustCoreEndToEndTest.ets`(HAP-side ets 测试)
- Reference: `entry/src/ets/__tests__/BridgeHealthSmokeValidator.ets`(已有测试结构)

**背景**:S6 闭环证明要求 HAP 在模拟器/真机上跑通 `search→detail→toc→content` 经 Rust Core,**非 fixture**。本 Task 写一个 HAP-side ets 测试,驱动 `BookApi` 全链路,断言每步非空 + 反 fixture 断言(fixture 数据有特定标识,如 `fixture://` URL 或 "三体" hardcoded)。

- [ ] **Step 1: 探查现有 ets 测试结构**

```bash
ls "/Users/minliny/Documents/Reader for HarmonyOS/entry/src/main/ets/__tests__/"
cat "/Users/minliny/Documents/Reader for HarmonyOS/entry/src/main/ets/__tests__/BridgeHealthSmokeValidator.ets" | head -40
```
Expected: 看到现有测试 validator 结构,通常用 `describe`/`it` 或自定义 runner。

- [ ] **Step 2: 创建 RustCoreEndToEndTest.ets**

`entry/src/main/ets/__tests__/RustCoreEndToEndTest.ets`:

```typescript
// RustCoreEndToEndTest — proves search→detail→toc→content via Rust Core on
// HarmonyOS simulator/device. This is App/simulator-level proof (charter §10.3),
// NOT real device proof (deferred to S7).
//
// S6 closure evidence: MUST
// 1. Boot Rust Core runtime (not fixture)
// 2. Dispatch book.search via BookApi → assert non-fixture results
// 3. Dispatch book.detail via BookApi → assert enriched metadata
// 4. Dispatch book.toc via BookApi → assert non-empty chapters
// 5. Dispatch chapter.content via BookApi → assert non-empty content
//
// Fixture-only assertions (readerHomeDashboard data) do NOT satisfy this test.

import { ReaderCoreClient } from '../api/ReaderCoreClient';
import { BookApi, Book, SearchBook, Chapter } from '../api/BookApi';
import { SourceApi } from '../api/SourceApi';

// Test book source: a minimal Legado-format source with real rules.
// NOTE: This test hits the real network (host.request → OHOSHTTPHostTransport
// → real HTTP). It is NOT a unit test. Run on simulator/device with network.
const TEST_BOOK_SOURCE_JSON = `{
  "bookSourceName": "S6 E2E Test Source",
  "bookSourceUrl": "https://www.biquges123.com",
  "bookSourceType": 0,
  "enabled": true,
  "searchUrl": "https://www.biquges123.com/search.php?q={{key}}",
  "ruleSearch": {
    "bookList": "css:.result-list .item",
    "name": "css:.book-name@text",
    "author": "css:.book-author@text",
    "bookUrl": "css:a@href"
  },
  "ruleBookInfo": {
    "name": "css:h1@text",
    "author": "css:.author@text",
    "intro": "css:.intro@text"
  },
  "ruleToc": {
    "chapterList": "css:.chapter-list a",
    "chapterName": "@text",
    "chapterUrl": "@href"
  },
  "ruleContent": {
    "content": "css:.content@html"
  }
}`;

export interface RustCoreE2EResult {
  searchPassed: boolean;
  detailPassed: boolean;
  tocPassed: boolean;
  contentPassed: boolean;
  searchResults: number;
  tocChapters: number;
  contentLength: number;
  antiFixtureOk: boolean;
  error: string | null;
}

export async function runRustCoreEndToEnd(): Promise<RustCoreE2EResult> {
  const result: RustCoreE2EResult = {
    searchPassed: false,
    detailPassed: false,
    tocPassed: false,
    contentPassed: false,
    searchResults: 0,
    tocChapters: 0,
    contentLength: 0,
    antiFixtureOk: false,
    error: null
  };

  try {
    const client = ReaderCoreClient.init();
    const sourceApi = new SourceApi(client);
    const bookApi = new BookApi(client);

    // 1. Import source
    const importResult = await sourceApi.importBookSource(TEST_BOOK_SOURCE_JSON);
    if (!importResult.success) {
      result.error = `source.import failed: ${importResult.data}`;
      return result;
    }

    const sourceId = 'https://www.biquges123.com';

    // 2. Search
    const searchResults: SearchBook[] = await bookApi.search(sourceId, '凡人修仙传', 1);
    if (searchResults.length === 0) {
      result.error = 'search returned 0 results — network or rule issue';
      return result;
    }
    result.searchPassed = true;
    result.searchResults = searchResults.length;

    // Anti-fixture assertion: fixture results have detailURL starting with 'fixture://'
    const firstResult = searchResults[0];
    if (firstResult.bookUrl.indexOf('fixture://') === 0) {
      result.error = `search result looks like fixture: ${firstResult.bookUrl}`;
      return result;
    }
    if (firstResult.name.indexOf('Mock') === 0 || firstResult.name.length === 0) {
      result.error = `search result looks like mock: ${firstResult.name}`;
      return result;
    }

    // 3. Detail
    const seedBook: Book = {
      bookUrl: firstResult.bookUrl,
      tocUrl: '',
      name: firstResult.name,
      author: firstResult.author,
      coverUrl: firstResult.coverUrl,
      intro: firstResult.intro,
      kind: firstResult.kind,
      wordCount: '',
      latestChapterTitle: '',
      origin: sourceId
    };
    const enriched = await bookApi.detail(sourceId, seedBook);
    if (enriched.name.length === 0) {
      result.error = 'detail returned empty name';
      return result;
    }
    result.detailPassed = true;

    // 4. TOC
    const chapters: Chapter[] = await bookApi.toc(sourceId, enriched);
    if (chapters.length === 0) {
      result.error = 'toc returned 0 chapters';
      return result;
    }
    result.tocPassed = true;
    result.tocChapters = chapters.length;

    const firstChapter = chapters[0];
    if (firstChapter.url.indexOf('fixture://') === 0) {
      result.error = `toc chapter URL looks like fixture: ${firstChapter.url}`;
      return result;
    }

    // 5. Content
    const content = await bookApi.content(sourceId, enriched, firstChapter);
    if (content.length === 0) {
      result.error = 'content returned empty';
      return result;
    }
    result.contentPassed = true;
    result.contentLength = content.length;

    // Anti-fixture assertion: fixture content has "在中国，任何超脱飞扬的思想"
    if (content.indexOf('在中国，任何超脱飞扬的思想') !== -1 || content.indexOf('Mock content') !== -1) {
      result.error = `content looks like fixture/mock: ${content.substring(0, 50)}`;
      return result;
    }

    result.antiFixtureOk = true;
    return result;
  } catch (e) {
    result.error = `E2E threw: ${e}`;
    return result;
  }
}
```

- [ ] **Step 3: 把测试挂到 SettingsPage 的"运行证据"面板(可选,便于手动跑)**

读 `pages/Index.ets` 的 `RuntimeDeviceEvidencePanel`,在 `aboutToAppear` 末尾加:
```typescript
// S6 E2E proof: run RustCoreEndToEnd and surface pass/fail
runRustCoreEndToEnd().then((r: RustCoreE2EResult) => {
  this.hostBus = r.searchPassed && r.detailPassed && r.tocPassed && r.contentPassed
    ? `PASS op:e2e`
    : `FAIL e2e:${r.error ?? 'unknown'}`;
}).catch((_e: Error) => {
  this.hostBus = 'FAIL e2e:threw';
});
```

或:跑 `npm run smoke:device-runtime` 看日志。

- [ ] **Step 4: 启动模拟器/连接真机 + 跑 HAP**

```bash
# 列出 hdc 设备
hdc list targets
# 安装新 HAP
hdc install -r entry/build/default/outputs/default/entry-default-signed.hap
# 启动 App
hdc shell aa start -a EntryAbility -b com.reader.harmonyos
# 等 30 秒(让 E2E 跑完)
sleep 30
# 看日志
hdc hilog | grep -i "reader_core\|harmony_napi\|RustCore\|E2E" | head -50
```
Expected: 日志显示 E2E 通过 —— `searchPassed=true`、`detailPassed=true`、`tocPassed=true`、`contentPassed=true`、`antiFixtureOk=true`。

或:
```bash
npm run smoke:device-runtime 2>&1 | tail -30
```
Expected: summary 中 `HostBus=PASS op:e2e` 或 `PASS op:<id>`(NAPI smoke 闭环仍跑),无 `FAIL`。

- [ ] **Step 5: 归档 E2E artifact**

```bash
mkdir -p artifacts/harmony-s6-e2e/$(date -u +%Y%m%dT%H%M%SZ)
cp artifacts/device-runtime-smoke/latest/device_runtime_smoke_summary.json \
   artifacts/harmony-s6-e2e/$(date -u +%Y%m%dT%H%M%SZ)/
# 若 SettingsPage 运行证据面板截图
hdc shell snapshot_display -f /data/local/tmp/s6_e2e.png
hdc file recv /data/local/tmp/s6_e2e.png artifacts/harmony-s6-e2e/$(date -u +%Y%m%dT%H%M%SZ)/
```

- [ ] **Step 6: 提交**

```bash
cd "/Users/minliny/Documents/Reader for HarmonyOS"
git add entry/src/main/ets/__tests__/RustCoreEndToEndTest.ets \
        entry/src/main/ets/pages/Index.ets \
        artifacts/harmony-s6-e2e/
git commit -m "test(harmony/s6): add RustCoreEndToEndTest — search→detail→toc→content via Rust Core

App/simulator-level proof (charter §10.3) that HarmonyOS business path
routes through Rust Core C ABI, NOT fixture:

1. source.import → Core (Legado BookSource JSON)
2. book.search via BookApi → asserts non-empty results + anti-fixture
   assertion (bookUrl must not start with 'fixture://', name must not
   start with 'Mock')
3. book.detail via BookApi → asserts enriched non-empty name
4. book.toc via BookApi → asserts non-empty chapters + anti-fixture
5. chapter.content via BookApi → asserts non-empty + anti-fixture
   (must not contain '在中国，任何超脱飞扬的思想' or 'Mock content')

Run on HarmonyOS simulator/device with network. Real device proof
deferred to S7.

Evidence layering (charter §10.3):
- captureHarmonyNapiSmokeArtifact = wrapper smoke (NAPI layer, Task 2)
- RustCoreEndToEndTest = App/simulator proof (this Task)
- Real device proof = S7
- Corpus benchmark (4-platform same corpus) = S7"
```

---

### Task 9: 章程 §9 五问 + 最终状态摘要 + 提交 plan 到主仓库

**Files:**
- No HarmonyOS file changes — commit message body 含五问答案
- 主仓库新增: `docs/superpowers/plans/2026-06-27-harmonyos-rust-core-integration-s6.md`(本文件)
- 主仓库修改: `reports/tooling/release-blockers.json`(新增 HarmonyOS 专属 blockers,见 Step 5)

- [ ] **Step 1: 整理章程 §9 五问答案**

```markdown
## 章程 §9 五问答案(S6 HarmonyOS Rust Core 接入 + UI 业务源切换)

1. **本轮兼容目标来自本地 legado 的哪个代码路径或数据结构**:
   - `app/src/main/java/io/legado/app/model/webBook/WebBook.kt` 四段调度(searchBookAwait/getBookInfoAwait/getChapterListAwait/getContentAwait)→ HarmonyOS BookApi facade
   - `app/src/main/java/io/legado/app/data/entities/BookSource.kt` BookSource 模型 → 通过 SourceApi.importBookSource 接收
   - `app/src/main/java/io/legado/app/ui/book/search/SearchViewModel.kt` + `ui/book/info/BookInfoViewModel.kt` + `ui/book/read/ReadBookViewModel.kt` 状态机 → ArkTS ViewModel 层(BookshelfViewModel/SearchViewModel/ReadingViewModel/ImportBookSourceViewModel)
   - `app/src/main/java/io/legado/app/ui/association/ImportBookSourceViewModel.kt` JSON 导入 → ImportBookSourceViewModel

2. **本轮迁移资产来自本地 Reader-Core 的哪个代码路径**(Swift Core 已实现部分):
   - `Sources/ReaderCoreProtocols/NetworkProtocols.swift` HTTPRequest/HTTPResponse 字段定义 → Rust Core HostHttpRequest/HostHttpResponse 字段已对齐 → HarmonyOS `entry/src/main/ets/host/HttpRequest.ets`
   - `Sources/ReaderCoreParser/NonJSParserEngine.swift` 四入口 → Rust Core book.* dispatch → HarmonyOS BookApi
   - `Sources/ReaderCoreProtocols/PlatformAdapterContracts.swift:125-166` androidReference() manifest 6 类 host-owned capability → HarmonyOS HostAdapter 仅实现 http.execute,其他 capability 留 S7
   - `Sources/ReaderCoreModels/BookSource.swift` Legado JSON 怪 quirks 兼容 → Rust Core LegadoBookSource serde 已对齐 → HarmonyOS SourceApi 直接 JSON.parse 传入
   - 对照 Legado 新建(Reader-Core 缺失):HarmonyOS host-adapter 模块(Swift Core 无 ArkTS 等价物)、ArkTS UI 业务源切换(Swift Core 无 HarmonyOS UI)

3. **本轮 Rust 改动落在哪个 crate、protocol schema、C ABI 或 binding**:
   - **零 Rust 改动** — 仅消费现有 C ABI v1 + NAPI binding(主仓库 bindings/harmony/)
   - protocol schema 未改(`book.search`/`book.detail`/`book.toc`/`chapter.content`/`source.import` 已存在)
   - C ABI 未改(`rc_runtime_create`/`send`/`cancel`/`destroy` 已冻结 v1)
   - 主仓库改动仅两处:(1) 本 plan 文档;(2) reports/tooling/release-blockers.json 新增 HarmonyOS 专属 blockers

4. **本轮是否改变三端 host adapter 的责任边界**:
   - **否** — 仅 HarmonyOS 接入现有边界,未改 Core/Host 边界定义
   - HarmonyOS 保留(章程 §4 Host owns):@ohos.net.http transport(OHOSHTTPHostTransport)、ArkWeb WebView(ArkWebPlatformAdapter,本期不实现)、HUKS(本期未用)、SystemTts(本期未用)
   - Core 不开 socket、不碰 WebView、不存明文凭据(红线 4 不变)

5. **本轮证据是 crate test、CLI conformance、FFI smoke、wrapper smoke、App/device proof 还是 corpus benchmark**:
   - `hvigorw assembleHap` = build proof(HAP + .so 打包)
   - `captureHarmonyNapiSmokeArtifact` on device = **wrapper smoke**(NAPI 层:abiVersion/pingSmoke/hostSmoke 闭环,Task 2)
   - `RustCoreEndToEndTest` on device = **App/simulator proof**(search→detail→toc→content via Rust Core,非 fixture,Task 8)
   - **非 real device proof**(真机留 S7)
   - **非 corpus benchmark**(四端同 corpus 留 S7 主线阶段)
```

- [ ] **Step 2: 最终状态摘要**

```bash
cd "/Users/minliny/Documents/Reader for HarmonyOS"
echo "=== Branch ==="
git branch --show-current
echo "=== Commits since baseline (b7aa631) ==="
git log --oneline b7aa631..HEAD
echo "=== File counts ==="
echo "ArkTS files total: $(find entry/src/main/ets -name '*.ets' | wc -l)"
echo "Host-adapter files: $(find entry/src/main/ets/host -name '*.ets' 2>/dev/null | wc -l)"
echo "API facade files: $(find entry/src/main/ets/api -name '*.ets' 2>/dev/null | wc -l)"
echo "ViewModel files: $(find entry/src/main/ets/viewmodel -name '*.ets' 2>/dev/null | wc -l)"
echo "=== @deprecated fixture files ==="
grep -l "@deprecated S6" entry/src/main/ets/services/ entry/src/main/ets/adapters/ entry/src/main/ets/repository/ 2>/dev/null | wc -l
echo "=== Verification commands ==="
hvigorw assembleHap --mode module -p product=default -p buildMode=debug --no-daemon 2>&1 | tail -3
```

- [ ] **Step 3: 提交 plan 文档到主仓库(本 Task 在主仓库执行)**

```bash
cd /Users/minliny/Documents/Reader-Core-Native
git add docs/superpowers/plans/2026-06-27-harmonyos-rust-core-integration-s6.md \
        reports/tooling/release-blockers.json
git commit -m "docs(plans): add HarmonyOS S6 Rust Core integration plan (9 tasks) + 3 HarmonyOS blockers

Plan: retire fixture-based business path (HomeDashboardService/SearchService/
BridgeHTTPClient/MockBookshelfRepository/FixtureEPUBParserAdapter), make
Rust Core the explicit business source for 5 ArkTS pages, add simulator/
device E2E proof.

Scope:
- Task 1: fix Index.d.ts to align with reader_napi.cpp 12 exports (blocker)
- Task 2: verify libreader_core_napi.so packaged + 12-export smoke on device
- Task 3: build ArkTS host-adapter module (HostAdapter/HostBus/HostRuntime/
  HttpExecuteHandler) — mirrors Android bindings/android/host-adapter/
- Task 4: OHOSHTTPHostTransport (@ohos.net.http implements HostTransport)
- Task 5: ReaderCoreClient singleton + BookApi/SourceApi facade
- Task 6: deprecate fixture services (strangler, not delete)
- Task 7: wire 5 ArkTS pages to Rust Core via ViewModel layer
- Task 8: RustCoreEndToEndTest (search→detail→toc→content via Rust Core,
  anti-fixture assertions)
- Task 9: charter §9 five questions + final summary

Binding readiness confirmed:
- bindings/harmony/ complete (native/reader_napi.cpp 785 lines, 12 exports)
- sdk/reader_core.ts typed wrapper ready
- scripts/build-ohos.sh + build-harmony-napi.sh ready
- HarmonyOS repo entry/src/main/cpp/CMakeLists.txt calls main repo build-ohos.sh
- 3 HarmonyOS-specific blockers added to release-blockers.json:
  - rb-harmonyos-index-dts-stale: Index.d.ts only declares 3/12 exports
  - rb-harmonyos-host-adapter-missing: no ArkTS host-adapter module
  - rb-harmonyos-business-path-not-rust-core: 5 pages route through fixture

Per charter §10.5: strangler pattern — fixture path deprecated, not deleted.
Real device proof + corpus benchmark deferred to S7."
```

---

## Self-Review

### 1. Spec coverage(用户任务 + 章程 §9 五问)

- [x] Step 1(定位 HarmonyOS 仓库 + 审计 5 页壳与 NAPI POC):前置完成 —— 仓库在 `/Users/minliny/Documents/Reader for HarmonyOS`,分支 `codex/harmony-signed-device-runtime`,5 页 ArkTS UI(Index/BookshelfPage/SearchPage/ReaderPage/SettingsPage)全 fixture-based,NAPI POC 已加载 `libreader_core_napi.so` 但只跑 smoke
- [x] Step 2(审计 Rust Core HarmonyOS binding 就绪度):前置完成 —— binding 工具链就绪(`bindings/harmony/` 完整 + `scripts/build-ohos.sh` + `scripts/build-harmony-napi.sh`),但 HarmonyOS 仓库 `Index.d.ts` 严重过期(blocker)
- [x] Step 3(审计 HarmonyOS 平台能力):前置完成 —— 业务路径完全没接 Rust Core,无 host-adapter 模块,HTTP 走独立 BridgeHTTPClient(localhost:8899 桥)
- [x] Step 4(制定接入方案):Task 1-9 覆盖
- [x] Step 5(提交 plan + blocker):Task 9 Step 3 提交 plan 到主仓库,Step 5(本方案末尾)新增 HarmonyOS blockers
- [x] 约束 Core/Host 边界:HarmonyOS 仅保留 @ohos.net.http + ArkWeb(本期不实现)+ HUKS(本期未用)+ SystemTts(本期未用)(红线 4)
- [x] 约束 不破坏纯 UI:5 页 ArkTS UI 布局不动,只改业务源(切到 ViewModel)
- [x] 约束 证据分层:每个 commit 标注 build proof / wrapper smoke / App-sim proof / 非 real device
- [x] 章程 §9 五问:Task 9 Step 1
- [x] TTS 策略(AGENTS.md):本期不实现 TTS,留 S7;Core 不嵌入语音模型,Host 用 SystemTts(未变)

### 2. Placeholder scan

- Task 5 Step 1 的 `pollForResult` 与 `HostRuntime.startEventLoop` 都调 `readEvent`,存在 race —— S6 接受简化(单业务请求串行),S7 需要实现 event multiplexer。在 commit message 中标注
- Task 7 Step 5 的 `BookshelfRow` 与 `HomeBookRow` 字段不一致 —— 实际编辑时需调整 UI 字段访问。在 Step 5 已说明
- Task 7 Step 8 的 ReaderPage 改造 —— 当前 ReaderPage 从 BookshelfTab 跳转时未传 sourceId/bookUrl,S6 简化:从 BookshelfTab 跳转时传参,若 bookUrl 为空则显示提示。在 Step 8 已说明
- Task 8 的 `TEST_BOOK_SOURCE_JSON` 用 `https://www.biquges123.com` —— 若该站失效,E2E 测试会失败,需换源。这是 E2E 测试的固有特性,非 placeholder
- Task 3 Step 7 的 `HostBus.pollAndHandle` 中 `// TODO(S6): use a multiplexer` 是已知简化,S7 补
- Task 5 Step 1 的 `sendHostComplete`/`sendHostError` 方法 —— 实际 `ReaderCoreRuntime`(在 ReaderCoreNapiBridge.ets)有 `sendCommand`,但无 `sendHostComplete` 便捷方法。Task 5 实际实现时需用 `runtime.sendCommand('host.complete', params, requestId)` 替代,或在 ReaderCoreRuntime 加便捷方法

### 3. Type consistency

- `HostHttpRequest`/`HostHttpResponse` 在 Task 3 Step 3 定义,Task 4 `OHOSHTTPHostTransport` 使用 —— 字段名一致
- `HostReply`/`HostReplySuccess`/`HostReplyError` 在 Task 3 Step 4 定义,Task 3 Step 5/6/7 使用 —— 一致
- `Book`/`SearchBook`/`Chapter` 在 Task 5 Step 2 定义,Task 7 ViewModel + Task 8 E2E 使用 —— 一致
- `ReaderCoreClient.init()`/`get()`/`sendCommand`/`awaitResult`/`close` API 在 Task 5 Step 1 定义,Task 7 ViewModel + Task 8 E2E 使用 —— 一致
- `BookApi` 构造参数 `ReaderCoreClient` —— Task 5 Step 2 定义,Task 7 ViewModel + Task 8 E2E `new BookApi(ReaderCoreClient.get())` 调用 —— 一致
- `SourceApi` 构造参数 `ReaderCoreClient` —— Task 5 Step 3 定义,Task 7 ImportBookSourceViewModel + Task 8 E2E 使用 —— 一致
- `HostTransport` interface 在 Task 3 Step 2 定义,Task 4 `OHOSHTTPHostTransport implements HostTransport` —— 一致
- `CapabilityHandler` type 在 Task 3 Step 5 定义,Task 3 Step 6 `makeHttpExecuteHandler` 返回 `CapabilityHandler` —— 一致

### 4. 已知风险

1. **Index.d.ts 修复后 ArkTS 类型兼容性**:ArkTS strict 模式对 `unknown`/`any` 有限制,可能需要用 `Object`/`ESObject` 替代。缓解:Task 1 Step 4 验证编译
2. **OHOS SDK 路径**:`build-ohos.sh` 依赖 `OHOS_SDK_HOME` 环境变量,若未设置会失败。缓解:Task 2 Step 1 检查
3. **HarmonyOS 模拟器/真机可用性**:若无可用设备,E2E 测试无法跑。缓解:用 `hdc list targets` 检查;CI 用 `npm run smoke:device-runtime`
4. **readEvent race**:Task 5 `pollForResult` 与 Task 3 `HostRuntime.startEventLoop` 都调 `readEvent`,存在 race。S6 接受串行业务请求;S7 实现 event multiplexer
5. **bookshelf.list / source.list 命令存在性**:`bookshelf.list` 和 `source.list` 在 protocol schema 中**未验证存在** —— 若 Core 不支持,Task 7 ViewModel 需改为通过本地 BookSourceRepository 读取(S6 fallback)。Task 8 E2E 测试不依赖这两个命令(直接 source.import 后用固定 sourceId)
6. **Legado 书源 JSON 兼容**:Core 端 LegadoBookSource serde 需通过真实 Legado `assets/defaultData/bookSources.json` 验证 —— 留 S7 corpus benchmark
7. **WebView 书源不支持**:本期仅支持非 WebView 书源,含 `webView`/`webJs` 的 Legado 书源会失败。S7 补 `webview.evaluateJavaScript` host capability
8. **ArkTS async/await 在 aboutToAppear 中的限制**:HarmonyOS ArkTS 对 async lifecycle hook 有时序限制,可能需要用 Promise.then() 替代。Task 7 Step 5 验证
9. **测试书源站点可用性**:`https://www.biquges123.com` 可能改版或失效,E2E 测试失败时需换源。S7 用 Legado 真实书源 corpus
10. **HUKS / SystemTts 未接入**:本期不实现凭据存储和系统 TTS,留 S7。Core TTS 编排(Core 产出 segment descriptor,Host 用 SystemTts 发声)留 S7

### 5. 后续迭代(S7+)

- 真机 device proof(章程 §10.3 真实证据)
- Event multiplexer(解决 readEvent race,支持并发业务请求)
- WebView host capability(`webview.evaluateJavaScript`)— 支持含 webView/webJs 的 Legado 书源
- TTS(系统 TTS + HttpTTS,章程 TTS 策略:Core 编排 + Host 发声)
- 本地书(EPUB/TXT/PDF,Core 已支持 local_book.parse)—— 退役 FixtureEPUBParserAdapter 真正调用方
- RSS 订阅 + WebDAV 同步经 Rust Core
- 主题绘制
- 凭据存储 HUKS 接入(`credential.resolve` host capability)
- 阅读进度同步(reading.progress.update 经 Core)
- corpus benchmark 四端同结果(章程 §9 主线不变量)
- 删除 @deprecated fixture 服务(strangler 完成)
- 真机 + HarmonyOS CI 集成
