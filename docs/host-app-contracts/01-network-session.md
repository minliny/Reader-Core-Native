# 01 — Network / Session

> 主题域：HTTP transport、重定向、Cookie 策略与持久化、响应编码、响应体解析归属。
> 状态：🟡 契约已立（未实现）。本文件不声明任何实现完成。

## 1. 范围

**覆盖：**

- 远程阅读链路（search / detail / toc / chapter）所触发的 HTTP 请求与响应。
- 重定向策略的决策与执行边界。
- Cookie 的策略控制、jar 归属、持久化、出站注入、入站捕获。
- 响应字节流的编码检测与转换。
- 响应体的解析归属（HTML / XML / JSON）。
- 传输层错误的分类与映射。

**不覆盖（归其它主题域）：**

- WebView 登录交互、验证码、登录态注入 → 03 login/auth。
- SQLite schema、缓存、进度、下载队列持久化 → 02 local storage/sync。
- TXT/EPUB 本地文件读取 → 04 local book/files。
- TTS、UI、后台任务 → 05/06。

**上游事实来源：**

- `protocol/compatibility.md` §"HTTP Transport Capability"（现行 `http.execute`
  契约权威定义）。
- `ARCHITECTURE.md` §二 模块归属表、§3.4 Host Capability 走消息。
- `FEATURE_MATRIX.md` 能力归属总表（请求参数/重定向/Cookie 策略 → Rust Core；
  TLS·socket/HTTP Transport → Platform；响应编码 → Rust Core；
  Cookie/Session 持久化 → Rust Core）。

## 2. Capability inventory

| 子能力 | 归属类别 | 当前事实来源 |
|--------|----------|--------------|
| TLS / 实际网络 socket | **Host-owned** | ARCHITECTURE §二、FEATURE_MATRIX |
| HTTP 请求执行（实际 fetch） | **Shared-contract**（`http.execute`） | compatibility.md §HTTP Transport |
| 请求参数构建（url/method/headers/body/charset） | **Core-owned** | FEATURE_MATRIX（请求参数构建 → Core） |
| 重定向策略决策 | **Core-owned** | FEATURE_MATRIX（重定向策略控制 → Core） |
| 重定向执行（是否跟随、上限） | **Shared-contract** | ARCHITECTURE §3.4（`followRedirects` 参数） |
| Cookie 策略控制 | **Core-owned** | FEATURE_MATRIX（Cookie 策略控制 → Core） |
| Cookie jar 与持久化 | **Core-owned** | FEATURE_MATRIX（Cookie/Session 持久化 → Core） |
| 出站 Cookie 注入 / 入站 Set-Cookie 捕获 | **Shared-contract** | 本文件立约（现行协议未定义） |
| WebView Cookie 获取 | **Host-owned**（获取）→ 经 Shared-contract 注入 Core | ARCHITECTURE §二（平台获取 / Core 协议） |
| 响应编码检测与转换 | **Core-owned** | FEATURE_MATRIX（响应编码检测和转换 → Core） |
| 响应体解析（HTML/XML/JSON） | **Core-owned** | FEATURE_MATRIX（HTML/XML/JSON 解析 → Core） |
| 响应状态 / headers / finalUrl 透传 | **Shared-contract** | 本文件立约（现行协议仅"诊断允许"） |
| 传输层错误分类（timeout/DNS/TLS/4xx/5xx） | **Shared-contract** | 本文件立约（现行协议无错误码） |
| HTTP/2 push、QUIC 调优、代理 UI、证书钉选 UI | **Out-of-scope** | V1 不交付 |

## 3. Contracts

现行 `http.execute` 契约（compatibility.md）为：

```json
// host.request params
{ "url": "...", "method": "GET", "headers": {}, "body": null }
// host.complete result（Core v1 仅消费 body）
{ "status": 200, "body": "<string>" }
```

下列契约草案在现行基础上扩展，并显式标注 **gap**。所有扩展都属于
*协议 schema 变更*，必须由 protocol schema owner 评估是否触发 protocol
version bump；本文件不单方面宣告变更。

### 3.1 请求参数（Core → Host）

```json
{
  "url": "https://example.test/path",
  "method": "GET",
  "headers": { "User-Agent": "...", "Cookie": "..." },
  "body": null,
  "followRedirects": false,
  "maxRedirects": 0,
  "usePlatformCookieJar": false
}
```

- `headers`（含 `Cookie`）由 **Core-owned** 的请求构建与 cookie jar 解析后
  注入；host 必须原样发送，不得增删 Core 提供的 `Cookie` 头。
- `followRedirects` / `maxRedirects`：Core-owned 策略，host 执行。
  默认 `false`，即 host 不得自动跟随 3xx。
- `usePlatformCookieJar`：默认 `false`。host 不得使用平台自带 cookie jar
  除非显式置 `true`（V1 预期始终 `false`，保留字段以备诊断）。

**Gap A（params 缺字段）：** 现行 `http.execute` params 仅
`{url, method, headers, body}`，缺 `followRedirects` / `maxRedirects` /
`usePlatformCookieJar`。ARCHITECTURE §3.4 已示意这些字段，但未落入
`protocol/compatibility.md` 的权威契约。→ 后续 owner: protocol schema。

### 3.2 响应结果（Host → Core）

```json
{
  "status": 200,
  "finalUrl": "https://example.test/final",
  "headers": { "Content-Type": "text/html; charset=gbk", "Set-Cookie": ["..."] },
  "bodyBase64": "...",
  "charsetHint": "gbk"
}
```

- `bodyBase64`：**原始字节**，由 Core 做编码检测与转换（Core-owned）。
- `finalUrl`：重定向后的最终 URL（即便 `followRedirects=false`，也用于 Core
  判断是否发生 3xx）。
- `headers`：必须包含 `Content-Type` 与 `Set-Cookie`（多值用数组）。
- `charsetHint`：host 可选提供，但 **最终编码决策权在 Core**。

**Gap B（body 类型与编码归属冲突）：** 现行 result 的 `body` 是 **string**。
若 host 把字节解码成 string，则编码决策被 host 偷走，与
"响应编码检测和转换 → Core" 冲突。改为 `bodyBase64` 是 **protocol-breaking**，
会影响现有 `cli-host-http-smoke` 与 `protocol/fixtures/conformance/host/`
fixture。→ 后续 owner: protocol schema（须评估 version bump）。

**Gap C（finalUrl / Set-Cookie 未契约化）：** 现行协议称 status/headers/
finalUrl "additional fields allowed for diagnostics, Core v1 only consumes
body"。但 Core 要做重定向判断与 cookie 捕获，必须消费 `finalUrl` 与
`Set-Cookie`，不能停留在"诊断允许"。→ 后续 owner: protocol schema。

### 3.3 错误（Host → Core）

```json
// host.error params
{
  "operationId": 9021,
  "error": {
    "code": "HTTP_TRANSPORT_TIMEOUT",
    "message": "connect timed out",
    "retryable": true,
    "details": { "phase": "connect", "url": "https://example.test/path" }
  }
}
```

建议传输层错误码（host 产生，Core 映射到结构化 `CoreError`）：

| host error code | 含义 | retryable 默认 |
|-----------------|------|----------------|
| `HTTP_TRANSPORT_TIMEOUT` | 连接/读取超时 | true |
| `HTTP_TRANSPORT_DNS` | DNS 解析失败 | true |
| `HTTP_TRANSPORT_TLS` | TLS 握手/证书失败 | false |
| `HTTP_TRANSPORT_CONNECT` | 连接被拒/网络不可达 | true |
| `HTTP_TRANSPORT_HTTP_STATUS` | 4xx/5xx（当规则要求按状态判失败） | 视规则 |
| `HTTP_TRANSPORT_CANCELED` | 已被 Core cancel（host 不应主动发，由 Core 发 CANCELLED） | false |

**Gap D（无传输错误码）：** 现行 `host.error` 只复用 `CoreError`，未定义
传输层错误码集合，三端 host 会用各自原生异常字符串，Core 无法稳定映射。
→ 后续 owner: protocol schema + Core runtime。

### 3.4 Cookie jar 边界

- **Core-owned**：cookie jar 的存储、过期、domain 匹配、持久化（Core SQLite）。
- **Host-owned**：从 WebView 读取 cookie（见 03 login/auth 的 WebView 契约）。
- **Shared-contract**：Core 把出站 cookie 注入 `headers.Cookie`；host 在
  `result.headers.Set-Cookie` 回传入站 cookie；Core 解析并写入 jar。
- host 不得在 `usePlatformCookieJar=false` 时读写平台 cookie 存储。

**Gap E（WebView cookie 注入 Core 的命令缺失）：** 现行协议无
`session.setCookies`（或等价）命令把 host 获取的 WebView cookie 写入 Core
jar。该命令跨 01/03 两个主题域，在 03 立约时最终确定。→ 后续 owner:
protocol schema + 03 login/auth。

## 4. 验收证据要求

> 以下为 *契约成立所需的证据*，不是实现完成的声明。任何"已完成"措辞
> 仅指证据本身存在，不指实现已交付。

1. **Conformance fixture**：`protocol/fixtures/conformance/host/` 下存在
   覆盖扩展后 `http.execute` 请求与 `host.complete` 结果（含 `finalUrl`、
   `Set-Cookie`、`bodyBase64`）的 fixture，且被 `reader-contract` 测试解析
   通过。
2. **重定向服从证据**：三端 host adapter 各提供一条冒烟日志，展示
   `followRedirects=false` 时不自动跟随 3xx，并返回 `finalUrl`。
3. **Cookie jar 隔离证据**：三端 host adapter 各提供一条冒烟日志，展示
   `usePlatformCookieJar=false` 时不读写平台 cookie 存储，且出站 `Cookie`
   头与入站 `Set-Cookie` 由 Core 提供/解析。
4. **编码归属证据**：一条用例展示 host 返回 `bodyBase64` + `charsetHint`，
   Core 据此产出正确解码文本（证明编码决策在 Core）。
5. **可取消性证据**：`runtime.status` 快照展示 `http.execute` operation 的
   pending→cancelled 状态迁移，且原请求收到 `CANCELLED` 事件。
6. **三端一致性证据**：同一 Core 请求 + 同一固定响应，经 iOS / Android /
   Harmony 三端 host adapter，产出相同 canonical response DTO（对应
   ARCHITECTURE §四 退出条件）。

## 5. Risks

- **三端 HTTP 栈默认行为漂移**：OkHttp / URLSession / Harmony 网络组件在
  重定向、cookie、gzip、编码默认值上各不相同；无显式契约则静默漂移，
  违反"同一请求三端同 DTO"。
- **body string→base64 是协议破坏**：现有 `cli-host-http-smoke`、
  conformance fixture、`reader-contract` 测试均假设 string body；变更需
  协调 version bump 与 fixture 同步。
- **Cookie jar split-brain**：若任一端漏判 `usePlatformCookieJar=false`，
  平台 jar 与 Core jar 状态分裂，登录态/章节解锁态不一致。
- **重定向循环 / 无上限**：`maxRedirects` 未定义则存在 redirect loop 风险。
- **编码信息丢失**：若 host 先解码再传 string，BOM / `Content-Type` charset
  / meta charset 信息丢失，Core 无法纠错。
- **错误码不可移植**：无统一传输错误码时，三端原生异常字符串无法稳定映射
  到 `retryable` 决策，影响规则 fallback。

## 6. Follow-up owners

| 后续工作 | 责任方 |
|----------|--------|
| 扩展 `http.execute` params/result/error schema（Gap A/B/C/D） | protocol schema owner |
| 评估 `body` string→base64 是否触发 protocol version bump | protocol schema owner |
| Core cookie jar + 编码检测 + 重定向策略 + 传输错误映射 | Core runtime owner |
| `session.setCookies` 命令（跨 01/03，Gap E） | protocol schema + 03 login/auth |
| iOS `HTTPHostAdapter` 实现 `followRedirects` / `usePlatformCookieJar` / `bodyBase64` | iOS adapter owner |
| Android `HTTPHostAdapter`（OkHttp transport）同上 | Android adapter owner |
| Harmony `HttpHostAdapter` 同上 | Harmony adapter owner |
| 同步更新 `protocol/fixtures/conformance/host/` 与 `reader-contract` 测试 | protocol schema + Core runtime |

---

*本文件立约于 `codex/goal-host-app-contracts`，基线 fb4c3a7。不声明实现完成。*
