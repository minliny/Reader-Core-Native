# Reader-Core 协议兼容性

> 当前最高优先级文档是 `docs/LOCAL_REPO_MIGRATION_DIRECTIVE.md`。本文只定义 Rust
> Core 的 protocol/C ABI 兼容规则；业务迁移路线以本地仓库迁移指令为准。

- **Protocol version:** 1
- **C ABI version:** 1
- **Runtime config schema 版本：** 1
- **Compatibility owner:** `crates/reader-contract`、`crates/reader-runtime`、
  `crates/reader-ffi`、`include/reader_core.h`

本文是当前 JSON protocol 与 C ABI 兼容性规则的权威说明。全量开发目标见
`docs/FULL_DEVELOPMENT_ROADMAP.md`。

## 版本规则

1. `include/reader_core.h` 的函数签名或 ABI 所有权规则发生不兼容变化时，递增
   C ABI version。
2. JSON command/event schema 出现不兼容变化时，递增 protocol version。
3. runtime config schema 出现不兼容变化时，递增 runtime config schema version。
4. wrapper 可以拒绝比自身支持版本更新的 protocol/ABI。
5. patch-level 行为修复不递增版本，但必须增加 conformance fixture 或 crate test。

## 向后兼容变化

以下变化通常不需要递增 protocol version：

- request `params` 增加 optional field。
- event 增加新的 `type` value，且旧 wrapper 可忽略。
- 增加新的 command `method`，且旧 wrapper 不需要理解。
- error 增加新的 optional metadata。
- capability advertisement 增加新 capability。

## 不兼容变化

以下变化必须递增版本或提供迁移层：

- 删除或重命名现有 command/event 字段。
- 改变字段类型、必填性或枚举语义。
- 改变 `requestId`、`operationId`、callback buffer ownership 规则。
- 改变 `host.complete` / `host.error` 与 pending host operation 的关联语义。
- 改变 C ABI 函数签名、callback ABI、status code 含义。

## ABI v1 所有权规则

ABI v1 暴露：

- `rc_abi_version`
- `rc_runtime_create`
- `rc_runtime_send`
- `rc_runtime_cancel`
- `rc_runtime_destroy`

事件由 callback 传给 host。callback 收到的 bytes 只在 callback 调用期间有效；
host 如果要异步保留，必须立即复制。Core 不要求 host 调用 `rc_buffer_free`，因为
ABI v1 没有这个函数。

## 命令信封

命令是 JSON object：

```json
{
  "protocolVersion": 1,
  "requestId": 1,
  "method": "runtime.ping",
  "params": {}
}
```

规则：

- `protocolVersion` 当前必须等于 `1`。
- `requestId` 必须是正整数。
- `method` 必须是非空 dotted identifier，不能有空 segment 或首尾空白。
- `params` 必须是 object；无参数时使用 `{}`。
- unknown field 在 conformance 中按 schema 规则拒绝。

## 事件信封

事件是 JSON object：

```json
{
  "protocolVersion": 1,
  "requestId": 1,
  "type": "runtime.result",
  "result": {}
}
```

常见事件：

- `runtime.result`
- `runtime.error`
- `host.request`
- `runtime.status`

Host wrapper 应忽略自己不理解的可选字段，但不能忽略未知必需语义。

## Host bus 语义

Core 通过 `host.request` 请求平台执行 host-owned capability，例如 `http.execute`。

Host 必须用普通 command 回复：

- `host.complete`
- `host.error`

`host.complete` / `host.error` 通过 `operationId` 关联 pending host operation。
未知 operation、`operationId == 0` 或状态不匹配必须返回 structured error。

ABI v1 没有单独的 `rc_host_complete`。host completion 仍然通过
`rc_runtime_send` 发送 JSON command。

## HTTP transport capability

`http.execute` 是 shared contract：

- Core 负责构造 request descriptor、correlation、retry/redirect/charset/cookie 的
  语义要求。
- Host 负责真实 HTTP/TLS/socket、平台网络策略、证书、代理、WebView/session 获取。

Core 不在当前架构内直接打开 socket。

## File/cache capability

`file.read` / `file.write` 和 `cache.get` / `cache.put` 是 host-owned
capability：

- Core 负责给出 opaque path、namespace、key、byte bounds、TTL 和文本/base64
  payload。
- Host 负责平台文件系统、沙盒路径解析、缓存后端、权限和实际 I/O。
- Host 不能从 path、namespace、key 或 payload 推断 Legado 规则语义，也不能把缓存命中
  转换成业务结果；只能通过 `host.complete` 返回协议 payload 或通过 `host.error`
  返回能力执行失败。

文本和二进制 payload 使用二选一字段表达：`content` / `contentBase64` 或
`value` / `valueBase64`。具体编码解释属于 Core 后续处理或上层 contract，平台只负责
忠实搬运。

### Cookie/log/time/system/persistence capability

`cookie.get`、`cookie.set`、`log.emit`、`time.now`、`system.info`、
`persistence.get` 和 `persistence.put` 是 Core 向 host 索取平台事实或执行副作用的
协议能力。

- Cookie 能力只按 Core 给出的 URL、domain、name、sessionId 和 cookie record 读写
  平台 cookie jar。
- Log 能力只发出 Core 提供的 level、message、target 和结构化 fields。
- Time 能力只返回平台时钟事实，包含 `unixMillis` 和 `iso8601`。
- System 能力只返回 Core 请求的系统信息 key/value 对象。
- Persistence 能力只按 Core 给出的 opaque namespace/key/value/revision 读取或写入。

Host 不得把这些参数解释为 Legado 规则、书源语义、阅读状态或章节业务结果；所有业务
解释都保留在 Core 内。Host completion 只能返回 schema 中定义的能力 payload。

## Runtime config

Runtime config JSON schema 位于 `protocol/reader-runtime-config.schema.json`。

Config 由 host 在 runtime create 时提供，目标用途包括：

- `dataDirectory`
- `cacheDirectory`
- `logLevel`
- capability flags

Config schema 是 Core contract。平台 wrapper 只能做类型安全封装，不能改变语义。

## Conformance fixture

Conformance fixture 位于 `protocol/fixtures/conformance/**`。`reader-cli` 通过：

```bash
cargo run -p reader-cli -- --conformance
```

执行这些用例。退出码非零表示协议不兼容或用例失败。

Host replay fixture 使用同一份 command/hostResult 形状支持录制、回放和比对：

```bash
cargo run -p reader-cli -- --host-record tests/fixtures/host_replay/request_session_search.json
cargo run -p reader-cli -- --host-replay tests/fixtures/host_replay/request_session_search.json
cargo run -p reader-cli -- --host-record-suite tests/fixtures/host_replay/remote_reading_e2e_suite.json
cargo run -p reader-cli -- --host-replay-suite tests/fixtures/host_replay/remote_reading_e2e_suite.json
```

`--host-record*` 执行 Core command，捕获 Core 发出的 `host.request`，用 fixture 内
`hostResult` 完成该 host operation，并输出带 `expectHostRequest` / `expectResult`
的可回放 fixture。`--host-replay*` 使用这些 expectation 对 Core 输出做精确比较。

新增或修改协议语义时，必须同步：

- schema
- Rust DTO
- runtime validation
- conformance fixture
- wrapper 文档

## Capability advertisement

`core.info` 返回当前 Core 支持的 protocol、ABI、runtime config 和 capability 列表。
wrapper 不应通过硬编码推断能力，而应优先读取 `core.info`。

## 发布约束

- Core-side conformance 通过不等于 App/device proof。
- wrapper smoke 不等于 corpus parity。
- 只有通过 `docs/FULL_DEVELOPMENT_ROADMAP.md` 定义的 CLI + 三端 benchmark，才能声明
  三端 Rust Core 迁移完成或 production readiness。
