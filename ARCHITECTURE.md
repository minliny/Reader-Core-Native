# Reader 三平台统一内核完整实施规划

先明确依据边界：当前我能直接核实 Android 仓库的公开结构。它已经采用 Reader-Core 的 JSON 级契约，包含 `CoreBridge`，非 UI 后端已基本完成，而 UI 集成仍待完成；其中不少 RSS、本地书、WebDAV、同步、缓存能力目前是 Android 独立实现。以下规划会据此安排 Android 的保留和退役范围。Reader-Core 的约 10 万行 Swift、7921 个测试、3796 个 sample 和 `NonJSRuleScheduler` 规模，采用你提供的数据，不假装已经逐文件读取另外三个仓库。

---

## 一、最终技术决策

唯一主路线：

> **Rust 统一 Reader-Core + 单一版本化 C ABI + JSON 消息协议 + 三个平台薄桥接层。**

```text
┌─────────────────────────────────────────────────────────┐
│ Platform Applications                                   │
│                                                         │
│  iOS SwiftUI    Android Compose    Harmony ArkTS        │
└────────┬──────────────────┬───────────────────┬─────────┘
         │                  │                   │
   Swift Wrapper      JNI C++ Shim      Node-API C++ Shim
         │                  │                   │
         └──────────────────┼───────────────────┘
                            │
                    reader_core.h / C ABI
                            │
┌───────────────────────────▼─────────────────────────────┐
│                    Rust Reader-Core                      │
│                                                          │
│  Domain / Rule / QuickJS / Parsing / Cache / DB          │
│  Progress / Recovery / Diff / Local Book / RSS / Sync   │
│                                                          │
│  Core-owned task and state runtime                       │
└───────────────────────────┬─────────────────────────────┘
                            │
                 Host Capability Messages
                            │
        HTTP / WebView / TTS / Secure Store / File Picker
```

Rust 已有 `aarch64-unknown-linux-ohos` 官方 Tier 2 with Host Tools 目标，可通过 `rustup` 安装预编译 target；但 OpenHarmony SDK 目前仍需手工配置 Clang、sysroot 和 Cargo linker。因此技术基础是真实存在的，但必须把 HarmonyOS 真机链路作为第一道工程门槛。

### 不采用的方案

| 方案 | 决策 |
| -------------------- | ----------------------------- |
| Swift 继续做三端内核 | 放弃，HarmonyOS 工具链和运行时路线风险过高 |
| C++ 做主内核 | 作为 Rust OHOS 地基失败后的保底，不作为首选 |
| Rust 直接深度绑定 NAPI/JNI | 不采用，平台接口变化会污染 Core |
| UniFFI 作为主接口 | 不采用，HarmonyOS 仍需自建桥接，无法形成唯一接口 |
| 每个平台保留独立后端 | 最终全部退役 |
| 迁移全部 7921 个测试 | 不做 |
| 逐行照抄 10 万行 Swift | 不做；按功能纵切迁移行为 |

---

# 二、"统一内核"的准确边界

统一内核不等于所有操作系统能力都必须用 Rust 重写。

统一的是：

* 业务状态；
* 规则语义；
* 数据模型；
* JavaScript 运行环境；
* 缓存和数据库；
* 阅读进度；
* 恢复、校验、diff；
* 远程请求策略；
* 本地书解析；
* RSS、WebDAV 和同步逻辑。

平台只负责提供操作系统能力。

## 模块归属表

| 能力 | Rust Core | 平台 Adapter |
| ----------------------------- | --------: | ---------: |
| Book / Chapter / Source 模型 | ✅ | |
| CSS / XPath / JSONPath / 正则规则 | ✅ | |
| `@` 链、变量、多字段、替换规则 | ✅ | |
| JS 执行和脚本辅助函数 | ✅ | |
| 请求参数、重定向、Cookie 策略 | ✅ | |
| TLS 和实际网络 socket | | ✅ |
| 响应编码、正文解析、标准化 | ✅ | |
| SQLite schema、缓存、进度 | ✅ | |
| TXT / EPUB 解析 | ✅ | |
| RSS 解析和订阅状态 | ✅ | |
| WebDAV 协议和冲突策略 | ✅ | |
| TTS 文本切片和播放队列 | ✅ | |
| 系统 TTS 发声 | | ✅ |
| 登录 WebView / 验证码交互 | | ✅ |
| WebView Cookie 导入 Core | ✅ 协议 | ✅ 获取 |
| 安全凭据存储 | | ✅ |
| 文件选择和沙箱授权 | | ✅ |
| UI、导航、主题、字体 | | ✅ |
| 后台任务和通知 | | ✅ |

这意味着 Android 的 OkHttp、iOS 的 URLSession、HarmonyOS 的网络组件可以继续作为 transport，但平台必须关闭或绕开自己的 Cookie、重定向和内容解析策略，让 Rust Core 保持最终语义控制。

---

# 三、核心接口设计

## 3.1 不建立大量对象句柄

不要为 Book、Chapter、Source 分别跨语言暴露对象。

C ABI 只暴露一个运行时句柄：

```c
typedef struct rc_runtime rc_runtime_t;
```

所有领域对象都使用稳定 ID 和 JSON DTO。

这样可以避免：

* Swift ARC、JVM GC、ArkTS GC 和 Rust ownership 互相干扰；
* ChapterHandle 被平台过早释放；
* NAPI finalizer 和后台任务竞争；
* JNI global reference 泄漏；
* ABI 因模型字段变化而频繁破坏。

## 3.2 单一消息入口

建议最终头文件只保留约 6～10 个函数：

```c
typedef void (*rc_event_callback)(
  void *context,
  const uint8_t *json,
  size_t json_length
);

uint32_t rc_abi_version(void);

int32_t rc_runtime_create(
  const uint8_t *config_json,
  size_t config_length,
  rc_event_callback callback,
  void *callback_context,
  rc_runtime_t **out_runtime
);

int32_t rc_runtime_send(
  rc_runtime_t *runtime,
  const uint8_t *command_json,
  size_t command_length
);

int32_t rc_runtime_cancel(
  rc_runtime_t *runtime,
  uint64_t request_id
);

void rc_runtime_destroy(rc_runtime_t *runtime);

void rc_buffer_free(uint8_t *buffer, size_t length);
```

Rust 官方 FFI 文档明确支持通过 `extern "C"`、无符号改名导出函数，并可生成 `staticlib` 或 `cdylib` 供外部语言调用。

## 3.3 JSON 控制协议

Android 当前已经采用 Reader-Core JSON-level contract，因此直接把现有契约升级成统一 Core Protocol，能避免重新设计三套绑定。

请求：

```json
{
  "protocolVersion": 1,
  "requestId": 10001,
  "method": "book.search",
  "params": {
    "sourceIds": ["source-1"],
    "keyword": "三体",
    "page": 1
  }
}
```

返回事件：

```json
{
  "protocolVersion": 1,
  "requestId": 10001,
  "type": "result",
  "data": {
    "books": []
  }
}
```

错误：

```json
{
  "protocolVersion": 1,
  "requestId": 10001,
  "type": "error",
  "error": {
    "code": "RULE_EVALUATION_FAILED",
    "message": "XPath evaluation failed",
    "retryable": false,
    "details": {}
  }
}
```

## 3.4 Host Capability 也走消息

不要设计一大套复杂函数指针。

当 Rust 需要平台能力时，发出事件：

```json
{
  "type": "host.request",
  "operationId": 9021,
  "capability": "http.execute",
  "params": {
    "url": "https://example.com",
    "method": "GET",
    "headers": {},
    "followRedirects": false,
    "usePlatformCookieJar": false
  }
}
```

平台完成后再发回 Core：

```json
{
  "method": "host.complete",
  "params": {
    "operationId": 9021,
    "result": {
      "status": 200,
      "headers": {},
      "bodyBase64": "..."
    }
  }
}
```

这种双向消息总线可以统一 HTTP、WebView 登录、文件选择、安全存储和 TTS，而且三端桥接层几乎完全同构。

---

# 四、Rust Core 目标目录

建议先建立新仓库 `Reader-Core-Native`。等三端全部切换后，再将旧 Swift 仓库归档，并把新仓库改名为 `Reader-Core`。

```text
Reader-Core-Native/
├── Cargo.toml
├── rust-toolchain.toml
├── crates/
│   ├── reader-contract/
│   │   ├── command.rs
│   │   ├── event.rs
│   │   ├── error.rs
│   │   └── schema.rs
│   │
│   ├── reader-domain/
│   │   ├── book.rs
│   │   ├── chapter.rs
│   │   ├── source.rs
│   │   └── progress.rs
│   │
│   ├── reader-rule/
│   │   ├── lexer.rs
│   │   ├── parser.rs
│   │   ├── scheduler.rs
│   │   ├── css.rs
│   │   ├── xpath.rs
│   │   ├── json_path.rs
│   │   ├── regex.rs
│   │   ├── chain.rs
│   │   └── variables.rs
│   │
│   ├── reader-js/
│   │   ├── runtime.rs
│   │   ├── context.rs
│   │   ├── promise.rs
│   │   ├── host_api.rs
│   │   └── polyfill/
│   │
│   ├── reader-content/
│   │   ├── search.rs
│   │   ├── detail.rs
│   │   ├── toc.rs
│   │   ├── chapter.rs
│   │   └── normalization.rs
│   │
│   ├── reader-storage/
│   │   ├── database.rs
│   │   ├── migrations.rs
│   │   ├── cache.rs
│   │   ├── progress.rs
│   │   └── download.rs
│   │
│   ├── reader-local-book/
│   │   ├── txt.rs
│   │   ├── epub.rs
│   │   └── encoding.rs
│   │
│   ├── reader-rss/
│   ├── reader-sync/
│   ├── reader-runtime/
│   │   ├── runtime.rs
│   │   ├── dispatcher.rs
│   │   ├── worker.rs
│   │   ├── cancellation.rs
│   │   └── host_bridge.rs
│   │
│   └── reader-ffi/
│       ├── lib.rs
│       ├── exports.rs
│       ├── panic_guard.rs
│       └── memory.rs
│
├── native/
│   ├── quickjs/
│   └── sqlite/
│
├── include/
│   └── reader_core.h
│
├── bindings/
│   ├── ios/
│   ├── android/
│   └── harmony/
│
├── tools/
│   └── reader-cli/
│
├── protocol/
│   ├── reader-command.schema.json
│   ├── reader-event.schema.json
│   └── compatibility.md
│
├── samples/
│   ├── selected/
│   └── index.json
│
└── scripts/
    ├── build-ios.sh
    ├── build-android.sh
    ├── build-ohos.sh
    ├── package-xcframework.sh
    └── generate-header.sh
```

---

# 五、运行时和并发模型

## 5.1 第一版不把 Tokio 设为基础设施

为了降低 OHOS 依赖适配风险，第一版采用：

* 一个 Core command dispatcher；
* 固定大小的 Rust worker pool；
* channel 传递任务；
* 每个请求有 cancellation token；
* HTTP 由平台异步执行；
* SQLite 使用受控的单写线程；
* JS 使用专属串行线程。

后续确认 OHOS 上 Tokio/mio 稳定后再考虑引入，不应让它成为项目第一道依赖风险。

## 5.2 QuickJS 模型

第一版只建立一个专用 JS 执行线程：

```text
Core workers
       │
       ▼
JS Task Queue
       │
       ▼
Single QuickJS Thread
       │
       ├── JSRuntime
       ├── JSContext pool
       └── Promise job pump
```

QuickJS 明确规定一个 `JSRuntime` 内不支持多线程；同时它不实现 ECMA‑402 `Intl`。因此不能让多个 Rust worker 同时操作同一 runtime，规则依赖的 `Intl` 能力必须通过 polyfill 或 Rust host function 补齐。

必须内置：

* 执行超时；
* 内存上限；
* interrupt handler；
* Promise job pump；
* `console`；
* `setTimeout` / `clearTimeout`；
* `TextEncoder` / `TextDecoder`；
* `atob` / `btoa`；
* `fetch` 或 Reader 专用请求 API；
* Crypto 常用函数；
* URL 和编码辅助函数；
* JS Error → CoreError 映射。

不要开放 QuickJS 自带的文件、进程和 OS 模块。

---

# 六、原有四个仓库如何处理

## 6.1 Reader-Core Swift

现在开始：

* 打 tag：`swift-core-freeze-before-rust`;
* 只修严重 bug；
* 不再新增未来需要跨平台的能力；
* 保留为算法参考和行为 oracle；
* 不迁移全部 XCTest；
* Rust 对应模块完成后，Swift 模块进入只读状态；
* 三端全部切换后归档。

不需要先清理 Swift Core，也不需要重构成完美架构再迁移。那会产生一次无收益的中间重构。

## 6.2 Reader-for-Android

Android README 显示当前结构包括 `ui/`、`data/model/`、`data/bridge/`、`data/adapter/`、`data/repository/`、`data/network/` 和 `data/storage/`，并已有 `CoreBridge`、OkHttp、Room、DataStore、RSS、本地书、TTS、WebDAV、同步和缓存能力。

按以下方式切：

| 当前 Android 部分 | 处理 |
| ------------------------ | ------------------------- |
| `ui/` | 保留 |
| `data/model/` | 暂时保留，最终由统一 JSON Schema 对齐 |
| `data/bridge/CoreBridge` | 保留接口 |
| CoreBridge 当前实现 | 替换为 `NativeCoreBridge` |
| OkHttp | 保留为 Host HTTP Transport |
| WebView | 保留为登录和验证码 Adapter |
| TTS | 保留系统发声 Adapter |
| SAF / 文件选择 | 保留 |
| DataStore | 只保留 UI 设置 |
| Room 内容数据库 | Rust DB 完成后退役 |
| HTML/XML parser | 移入 Rust |
| RSS parser | 移入 Rust |
| TXT/EPUB parser | 移入 Rust |
| WebDAV 和 sync 逻辑 | 移入 Rust |
| remote cache/offline | 移入 Rust |

Android 当前"fake/real dual mode"可以直接用于迁移：

```text
CoreBridge
├── FakeCoreBridge
├── LegacyKotlinCoreBridge
└── NativeCoreBridge ← 新增
```

最终只保留 `NativeCoreBridge`。

## 6.3 Reader-for-iOS

目标结构：

```text
Reader-for-iOS/
├── ReaderCoreClient.swift
├── ReaderCoreEvent.swift
├── ReaderHostAdapter.swift
├── HTTPHostAdapter.swift
├── WebViewLoginAdapter.swift
├── TTSHostAdapter.swift
└── ReaderCore.xcframework
```

迁移期间：

* SwiftUI 不改；
* ViewModel 接口尽量不改；
* 将原来直接调用 Swift Reader-Core 的 repository 替换为 `ReaderCoreClient`；
* URLSession 只作为 HTTP transport；
* Keychain 只作为 credential host；
* 文件选择和 TTS 留在平台层；
* 旧 Swift Core 在开发配置中可作为对照，但不进入最终包。

## 6.4 Reader-for-HarmonyOS

HarmonyOS 是首个平台验收目标。

目标结构：

```text
entry/
├── src/main/cpp/
│   ├── reader_napi.cpp
│   ├── reader_napi_runtime.cpp
│   ├── reader_event_dispatcher.cpp
│   ├── CMakeLists.txt
│   └── include/reader_core.h
│
├── src/main/ets/core/
│   ├── ReaderCore.ts
│   ├── ReaderCoreEvent.ts
│   ├── ReaderHostAdapter.ts
│   ├── HttpHostAdapter.ts
│   ├── WebViewHostAdapter.ts
│   └── TtsHostAdapter.ts
│
└── libs/arm64-v8a/
```

推荐把 Rust 编译为 `staticlib`，再静态链接进最终 NAPI `.so`：

```text
libreader_core.a
       +
reader_napi.cpp
       ↓
libreader_core_napi.so
```

这样 HAP 中只有一个主要 native module，不需要处理第二个 Rust `.so` 的加载顺序、RPATH 和动态依赖。

Rust 后台线程不能直接操作 ArkTS 值。C++ shim 必须通过线程安全回调机制把事件送回 ArkTS 所属环境；标准 Node-API 提供 `napi_threadsafe_function` 及其创建、调用和释放接口。

---

# 七、实施阶段与验收门槛

## 阶段 0：冻结方向与建立迁移清单

**工期：1 周**

任务：

1. 给四个仓库打迁移前 tag。
2. 停止三端新增重复后端。
3. 建立 `Reader-Core-Native`。
4. 从现有 JSON CoreBridge 整理首版 command/event schema。
5. 列出所有能力，标记：

   * Rust Core；
   * Platform Host；
   * 暂缓；
   * 退役。
6. 确定 V1 功能边界。

产物：

* `ARCHITECTURE.md`
* `FEATURE_MATRIX.md`
* `reader-command.schema.json`
* `reader-event.schema.json`
* `MIGRATION_MAP.md`

退出条件：

> 任意新功能都能明确判断应该写进 Core 还是平台 Adapter。

---

## 阶段 1：HarmonyOS Rust 地基

**工期：2～4 周**

任务：

1. 安装 `aarch64-unknown-linux-ohos` target。
2. 配置 OHOS SDK Clang wrapper 和 sysroot。
3. Rust 输出 `libreader_core.a`。
4. C++ NAPI 链接 Rust staticlib。
5. ArkTS 完成：

   * `createRuntime`
   * `send`
   * `cancel`
   * `destroy`
6. Rust 后台线程向 ArkTS 发送事件。
7. release HAP 签名和真机加载。
8. 验证后台、前台切换和 Ability 销毁。

退出条件：

* release HAP 真机可加载；
* 连续创建/销毁 runtime 不崩溃；
* Rust worker 能安全回调 ArkTS；
* ArkTS 销毁后不再收到悬空回调；
* panic、非法命令和取消均转成结构化错误。

**该阶段不通过，不开始大规模 Rust 业务迁移。**

---

## 阶段 2：统一 C ABI 和三端空壳接入

**工期：2～3 周**

任务：

* 实现 runtime handle；
* command/event dispatcher；
* request ID；
* cancel token；
* error taxonomy；
* core logging；
* iOS XCFramework；
* Android JNI `.so`；
* Harmony NAPI `.so`；
* 三端 `core.info` / `core.ping`。

退出条件：

> 三个平台加载同一 Core commit，并返回完全相同的协议版本、构建版本和能力列表。

---

## 阶段 3：规则内核和 QuickJS

**工期：4～8 周**

先实现高价值基础：

1. 正则；
2. JSONPath；
3. CSS selector；
4. XPath；
5. `@` chaining；
6. 多字段；
7. 替换规则；
8. 变量作用域；
9. `bookList` scoping；
10. `tag.index`；
11. QuickJS；
12. JS host API。

迁移方式：

* 按 Swift 模块语义翻译；
* 不重新设计规则语法；
* 不迁移全部测试；
* 从 3796 samples 中选 30～50 个最复杂样本；
* 使用 `reader-cli` 批量运行。

退出条件：

* 非 JS 和 JS 规则都能在 Harmony 真机执行；
* 复杂规则样本没有结构性缺口；
* JS 超时和内存上限生效；
* JS 异常不会导致进程崩溃。

---

## 阶段 4：远程阅读完整纵切

**工期：5～8 周**

只做一条真正完整的产品链：

```text
导入书源
  → 搜索
    → 书籍详情
      → 目录
        → 正文
          → 下一章
            → 缓存
              → 阅读进度
```

实现：

* source import；
* HTTP host contract；
* header、POST、redirect、Cookie；
* 编码检测和转换；
* HTML/XML/JSON 响应；
* 搜索规则；
* 详情规则；
* TOC 规则；
* content 规则；
* 内容清洗；
* chapter prefetch；
* 基础错误恢复。

接入顺序：

1. HarmonyOS；
2. Android；
3. iOS。

退出条件：

> 同一个书源、同一个请求和同一个章节，在三端经过同一个 Rust Core，得到相同的 canonical DTO。

到这里，架构已经真正成立。

---

## 阶段 5：统一数据库、缓存和进度

**工期：4～6 周**

Rust Core 负责：

* book source；
* bookshelf；
* chapter metadata；
* chapter content cache；
* reading progress；
* download queue；
* recent history；
* Cookie/session；
* schema migration；
* recovery metadata。

平台只传入：

```json
{
  "dataDirectory": "...",
  "cacheDirectory": "..."
}
```

因为项目尚未正式落地，不建议为现有各端测试数据库投入复杂迁移工具。允许在开发版本中清库重建。

退出条件：

* 三端数据库 schema 相同；
* App 重启后状态恢复；
* 离线能读已缓存章节；
* 阅读进度稳定；
* Android 不再使用 Room 保存 Core 内容数据。

此时形成第一个**三端统一内核成品版**。

---

## 阶段 6：补齐规则兼容面

**工期：6～12 周**

继续迁移：

* `NonJSRuleScheduler` 剩余行为；
* 复杂变量生命周期；
* 嵌套规则；
* 列表上下文；
* 链式回退；
* 批量规则；
* 特殊 URL；
* JS 网络和 Promise；
* 登录状态；
* Cookie 合并；
* retry/recovery；
* validation/diff。

不搬 XCTest，而是：

* 使用 sample corpus 批跑；
* 失败样本归类；
* 每修一类问题，就把该样本加入固定 smoke corpus；
* 最终保留约 100～300 个代表样本，而非复制 7921 个测试。

退出条件：

> `FEATURE_MATRIX.md` 中的规则能力全部达到三端共享实现，不存在 Swift/Kotlin/ArkTS 私有补丁。

---

## 阶段 7：本地书和扩展能力

**工期：6～10 周**

顺序：

1. TXT；
2. EPUB；
3. RSS；
4. TTS；
5. WebDAV；
6. backup/restore；
7. sync/conflict；
8. remote listing/offline；
9. interactive login；
10. 批量下载和后台恢复。

具体归属：

* TXT/EPUB/RSS parsing → Rust；
* TTS queue/feeder → Rust；
* 实际发声 → 平台；
* WebDAV/sync/conflict → Rust；
* HTTP → 平台 transport；
* 文件授权 → 平台；
* 文件内容 → Core 管理目录。

退出条件：

> Android README 当前列出的主要非 UI 能力均已迁移到 Rust Core 或明确归属于平台 Adapter。

---

## 阶段 8：退役重复后端和发布

**工期：4～6 周**

任务：

* 删除 Android 重复 parser/cache/sync；
* iOS 去除 Swift Reader-Core 运行依赖；
* HarmonyOS 只保留 ArkTS wrapper 和 C++ NAPI；
* release LTO；
* strip native symbols；
* debug symbol 单独归档；
* Core 版本显示；
* DB schema 版本；
* crash symbolication；
* 三端包体检查；
* App 生命周期和资源释放；
* release 构建和安装流程。

最终仓库状态：

```text
Reader-Core-Native       唯一业务内核
Reader-for-iOS           UI + Apple Host Adapters
Reader-for-Android       UI + Android Host Adapters
Reader-for-HarmonyOS     UI + Harmony Host Adapters
Reader-Core-Swift        archived
```

---

# 八、测试策略：不迁移测试体系

你的要求是正确的：不要把数月投入到 XCTest → Rust test 的形式迁移。

执行策略：

| 资产 | 处理 |
| --------------- | -------------------- |
| 7921 个 Swift 测试 | 原地保留，不翻译 |
| 3796 个 sample | 作为迁移语料池 |
| Swift Core | 行为参考，不继续扩展 |
| Rust 单元测试 | 只覆盖 FFI、内存、解析核心和关键边界 |
| 三端验证 | 少量真机 smoke flow |
| 规则兼容 | CLI 批量 sample runner |
| 网络验证 | 固定响应快照 + 少量真实书源 |

最低限度只保留四类检查：

1. FFI create/send/cancel/destroy 生命周期；
2. 30～50 个最复杂规则样本；
3. 搜索→详情→目录→正文完整链；
4. 三端 release 构建和真机启动。

这不是"测试迁移项目"，而是避免返工的最小开发门槛。

---

# 九、构建和产物规划

## iOS

```text
Rust staticlib
  → arm64 device
  → arm64/x86_64 simulator
  → XCFramework
  → Swift Clang module
```

产物：

```text
ReaderCore.xcframework
reader_core.h
module.modulemap
```

## Android

```text
Rust staticlib
       +
JNI C++ shim
       ↓
libreader_core_jni.so
```

首版：

* `arm64-v8a`；
* 后续加入 `x86_64` emulator。

Android Gradle 只负责：

* 调用 Core build；
* 打包 `.so`；
* 加载 JNI；
* 提供 Host Adapter。

## HarmonyOS

```text
Rust staticlib
       +
Node-API C++ shim
       ↓
libreader_core_napi.so
       ↓
HAP
```

首版只做 ARM64。

## 固定构建入口

```bash
./scripts/build-ios.sh
./scripts/build-android.sh
./scripts/build-ohos.sh
./scripts/package-all.sh
```

固定：

* Rust toolchain 版本；
* OHOS SDK 版本；
* Android NDK 版本；
* QuickJS commit；
* SQLite 版本；
* C ABI version；
* JSON protocol version。

---

# 十、关键风险和止损点

| 风险 | 优先级 | 处理 |
| -------------------------------- | --: | ---------------------------------- |
| OHOS Rust 依赖无法链接 | 最高 | 阶段 1 先验证；依赖最小化 |
| Rust staticlib 无法稳定进入 NAPI `.so` | 最高 | 只做最小链路，不先迁移业务 |
| QuickJS 与现有 JS 行为不一致 | 高 | 最复杂脚本先跑；按需 polyfill |
| HTTP 三端行为不同 | 高 | Core 控制 redirect、cookie、encoding |
| 规则迁移规模失控 | 高 | 按纵切交付，不按文件总量推进 |
| NAPI/JNI 生命周期泄漏 | 高 | 单 runtime handle + 消息协议 |
| Android 继续增长重复后端 | 高 | 立即冻结非 UI 后端 |
| 数据库三端漂移 | 中 | SQLite schema 归 Rust 管理 |
| 包体过大 | 中 | staticlib、LTO、strip、避免 ICU/OpenSSL |
| Rust panic 越过 FFI | 高 | 每个导出入口 panic guard，统一错误 |
| Scope creep | 高 | `FEATURE_MATRIX` 是唯一范围来源 |

## 三个硬止损点

### 止损点 A

Harmony release HAP 无法稳定加载并回调 Rust。

处理：

* 先尝试减少依赖、修 linker；
* 将有问题的底层依赖改为 C/C++；
* 仍失败才考虑 C++ Core；
* 不退回 Swift Harmony 方案。

### 止损点 B

QuickJS 无法兼容主要规则脚本。

处理：

* 增加 polyfill；
* 调整 host API；
* 必要时切换 QuickJS 分支/实现；
* 不允许三端使用不同 JS 引擎。

### 止损点 C

规则迁移量过大导致长期无成品。

处理：

* 停止横向迁移；
* 回到搜索→详情→目录→正文纵切；
* 每完成一条用户链路就接入三端；
* 不允许"Core 写了半年但 App 仍不能读书"。

---

# 十一、时间规划

以一名个人开发者为基准，建议看"有效全职周"，而不是日历周。

| 目标 | 有效全职工期 |
| --------------------------- | ---------: |
| Harmony Rust 地基 + 三端 ABI 空壳 | 5～8 周 |
| 三端远程阅读完整纵切 | 累计 14～22 周 |
| SQLite、缓存、进度、本地 TXT/EPUB | 累计 20～30 周 |
| 规则兼容面和主要扩展能力 | 累计 30～45 周 |
| 全部主要功能、退役重复后端、发布 | 累计 36～52 周 |

换算：

| 投入节奏 | 三端统一 Core 成品 V1 | 完整主要功能 |
| --------- | --------------: | -------: |
| 全职 | 5～8 个月 | 9～13 个月 |
| 每周约 25 小时 | 8～12 个月 | 14～20 个月 |
| 每周约 15 小时 | 12～18 个月 | 20～30 个月 |

其中"成品 V1"包括：

* 三端统一 Core；
* 书源导入；
* 搜索；
* 详情；
* 目录；
* 正文；
* 缓存；
* 阅读进度；
* TXT/EPUB 基础支持。

"完整主要功能"再包括：

* 广泛规则兼容；
* JS 高级能力；
* RSS；
* TTS；
* WebDAV；
* backup/sync；
* recovery/diff；
* interactive login；
* 离线和下载。

---

# 十二、前 30 天执行清单

## 第 1 周

* 四仓库打 tag；
* 建立 `Reader-Core-Native`；
* 建立 Rust workspace；
* 写 `reader_core.h` v0；
* 整理现有 CoreBridge JSON 命令；
* 写 feature matrix；
* 冻结平台后端扩展。

## 第 2 周

* 配置 OHOS Rust target；
* 输出最小 `libreader_core.a`；
* C++ NAPI 链接；
* ArkTS 调用 `core.info`；
* release HAP 真机安装。

## 第 3 周

* Rust worker thread；
* C++ thread-safe event dispatcher；
* ArkTS Promise / callback wrapper；
* create/send/cancel/destroy；
* Ability 销毁清理；
* 错误和 panic guard。

## 第 4 周

* Android JNI 接入 `core.info`；
* iOS XCFramework 接入 `core.info`；
* 三端统一 protocol version；
* QuickJS 在 Rust CLI 运行首个规则；
* QuickJS 在 Harmony 真机运行同一规则。

30 天的最终产物应当是：

```text
同一个 Rust Core commit
       │
       ├── iOS 能加载
       ├── Android 能加载
       └── HarmonyOS release HAP 能加载
       │
       都能发送异步命令
       │
       都能接收统一 JSON 事件
       │
       Harmony 能运行 QuickJS
```

如果 30 天后仍只是在编写架构文档或迁移普通 Swift 模型，而 Harmony 真机链路没有跑通，说明执行顺序已经偏离你的优先级。

---

# 十三、最终完成标准

项目只有同时达到以下条件，才算完成"完整适配 + 统一内核"：

1. 三端使用同一个 Rust Core commit。
2. 三端使用同一个 C ABI version。
3. 三端使用同一个 JSON protocol。
4. 三端使用同一个规则实现。
5. 三端使用同一个 QuickJS 环境。
6. 三端使用同一个数据库 schema。
7. 三端使用同一个缓存、进度、同步和 recovery 实现。
8. 平台桥接层不含规则、解析、缓存或业务状态逻辑。
9. Android 独立后端已退役。
10. iOS 不再依赖 Swift Reader-Core 运行逻辑。
11. HarmonyOS 是首个验收平台，而不是最后补适配。
12. Swift Core 只作为归档参考存在。

## 总结决策

**现在立即冻结三端重复后端，建立 Rust Core；先打穿 HarmonyOS Rust staticlib → C++ NAPI → ArkTS 的 release 真机链路，然后建立统一消息 ABI，再按"搜索—详情—目录—正文"的纵向链逐步迁移。**

这条路线没有把"最快"置于"统一内核"之前，但已经去掉了所有不直接服务于成品的工作：不迁移全部测试、不先清理 Swift、不重写 UI、不建立复杂 typed binding、不在 Core 内自建 TLS，也不维护长期双内核。
