# Reader for Android — Host Adapter

> 范围规则：本模块只在 `Reader for Android` 工作。它消费 Rust Core 的
> ABI/protocol，不新增也不修改 C ABI 或 Core 语义。Native 与其他平台不在此
> lane 修改。

## 角色

`HostAdapter` 是 Android host 侧适配器，把 Core 发出的 `host.request` event
桥接到 host 拥有的 capability（如 `http.execute`、`host.smoke.echo`），并把结果
编码回 `host.complete` / `host.error` command，交由现有
`ReaderCoreRuntime.send` / `rc_runtime_send` 发回 Core。

接入路径（host → Core ABI/protocol）：

```
Core (Rust) --rc_event_callback--> ReaderCoreRuntime.pollEvent (现有 Java wrapper)
   |                                   |
   |  host.request event bytes         |
   v                                   v
HostEventLoop.tick  -->  HostTransport.pollEventJson
   |                         |
   v                         v
HostRequest.parse  -->  HostAdapter.dispatch(capability)  -->  CapabilityHandler
                                                                   |
                                                                   v
                                          HostReply  <--  HostReplyCodec.encode
                                                                   |
                                                                   v
                                          HostTransport.sendCommand  -->  ReaderCoreRuntime.send
                                                                   |
                                                                   v
                                                            rc_runtime_send --> Core
```

本模块**不触碰 C ABI**：它消费协议（`host.request` / `host.complete` /
`host.error`），复用现有 `ReaderCoreRuntime` 的发送通道，不新增 native symbol。

## 组件

| 类 | 职责 |
| --- | --- |
| `HostBus` | host app 一站式接入点：`over(transport).register(...).start()/stop()`，含 daemon 轮询线程与同步 `tick`/`drain`。 |
| `HostEventLoop` | 闭环：poll → 过滤 `host.request` → dispatch → encode → send；忽略 result/error event。 |
| `HostTransport` | poll/send 抽象接口，使 loop 纯 JVM 可单测。 |
| `ReaderCoreHostTransport` | 生产 wiring：把 `HostTransport` 接到现有 `ReaderCoreRuntime`（JNI → C ABI）。 |
| `HostRequest` | 解析并校验 `host.request` event（operationId≥1、dotted capability、params）。 |
| `HostReply` | host 回复：`complete(resultJson)` 或 `error(code, message, retryable)`。 |
| `CapabilityHandler` | 单个 capability 的 host 侧实现接口。 |
| `HostAdapter` | 按 capability 分发；未注册/抛异常 → `host.error`。 |
| `HostReplyCodec` | 把 `HostReply` 编码为协议 command JSON。 |
| `HttpExecuteHandler` | `http.execute` shared-contract capability handler；委托 `HttpFetch` 做真实网络。 |
| `HttpFetch` / `HttpRequest` / `HttpResponse` | host-owned HTTP 机制抽象与请求/响应值对象。 |
| `HostSmokeEchoHandler` | `host.smoke.echo` conformance smoke capability；回显请求 params。 |
| `CredentialResolveHandler` | `credential.resolve` capability（host-app-contracts Gap D）；委托 `CredentialProvider` 解析凭据句柄。 |
| `CredentialProvider` / `Credential` | host-owned 凭据存储机制抽象与值对象（Keychain/Keystore）。 |
| `Json` | 零依赖最小 JSON codec，供纯 JVM 单测与 Android 嵌入。 |

## 构建 / 测试

需要 JDK 17 与 Gradle（已用 Gradle 9.5.1 验证）。

```bash
cd bindings/android/host-adapter
JAVA_HOME=<jdk17> gradle test          # 在线首跑拉取 JUnit Jupiter
JAVA_HOME=<jdk17> gradle --offline test # 依赖已缓存，可离线复跑
```

## Contract evidence

`src/test/resources/conformance/host/` 复制自 `protocol/fixtures/conformance/host/`。

- `HostReplyCodecTest` 断言 `HostReplyCodec` 输出与这些 fixture 在 canonical 形式下
  逐字节一致：`complete.json` ← `host.complete`，`error.json` ← `host.error`，
  `http-complete-with-metadata.json` ← 带 HTTP status/headers/body 的完成。
- `HostEventLoopTest` 用 fake `HostTransport` 端到端验证
  poll → parse → dispatch → encode → send 闭环，断言发出的 command 与 fixture 一致
  （modulo host 选择的 outbound requestId）；覆盖 result/error event 过滤、超时、
  malformed 请求、drain 多事件、outbound requestId 递增。
- `HostAdapterTest` 覆盖 dispatch → encode 端到端与各失败模式。
- `HttpExecuteHandlerTest` 用 fake `HttpFetch` 验证 `http.execute` 请求/响应契约
  （缺 url → 非重试 INTERNAL；fetch 抛异常 → 可重试 INTERNAL；带 headers 的完成与
  `http-complete-with-metadata.json` fixture 对齐），并经 `HostEventLoop` 端到端发命令。
- `ProtocolConformanceTest` **直接读取上游 `protocol/fixtures/conformance/host/`**
  （Gradle system property 注入路径），断言 codec 输出与
  `complete/error/http-complete-with-metadata/http-complete-invalid-status` fixture
  逐字节一致（modulo outbound requestId），拒绝 `*-operation-zero` 负 fixture，校验
  `request.json` smoke 参数形状与 invalid-capability 负 fixture —— 协议变更即断测。
- `HostBusTest` 覆盖产品 surface：同步 `tick`/`drain` 脚本、unsupported capability →
  `host.error`、`start`/`stop` 幂等，并用阻塞型 fake transport 驱动 daemon 轮询线程
  端到端验证异步 host.request 处理。
- `CredentialResolveHandlerTest` 用 fake `CredentialProvider` 验证 `credential.resolve`
  草案契约（填补 host-app-contracts Gap D）：解析 → `{username,password}`，未知 handle →
  非重试 INTERNAL，provider 抛异常 → 可重试 INTERNAL，并经 `HostEventLoop` 端到端发命令。

这是本 lane 每轮提交的可验证 contract evidence（Gradle `test` task，纯 JVM，无需
NDK/设备）。模块通过 `sourceSets` 编译引用现有 Java JNI wrapper（`ReaderCoreRuntime`
等），不修改 wrapper 源。

## 不在此 lane

- Gradle packaging 成完整 AAR / Android App 项目、UI、WebView、CookieManager、
  keystore、network policy 仍属 host-app 工作。
- C ABI 扩展（`rc_host_complete`、`rc_runtime_poll` 等）不在本 lane；host
  completion 仍通过 `rc_runtime_send` 发送普通 JSON command。
