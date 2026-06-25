# Reader-Core 协议兼容性

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
- 只有通过 `docs/FULL_DEVELOPMENT_ROADMAP.md` 定义的 corpus benchmark，才能声明
  Legado parity 或 production readiness。
