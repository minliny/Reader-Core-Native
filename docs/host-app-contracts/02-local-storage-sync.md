# 02：本地存储 / 同步

> 主题域：SQLite schema、缓存、阅读进度、下载队列、最近历史、WebDAV、
> 备份/恢复、同步/冲突解决。
> 状态：🟡 契约已立（未实现）。本文件不声明任何实现完成。

## 1. 范围

**覆盖：**

- Core 数据目录与缓存目录的归属、创建、迁移。
- SQLite schema 版本管理与迁移。
- 章节内容缓存、阅读进度、下载队列、最近历史的持久化。
- Cookie/Session 持久化（与 01 network/session 的 jar 边界互补）。
- 备份/恢复（export/import Core 全量状态）。
- WebDAV 远程同步与冲突解决。
- Recovery / 校验 / Diff 元数据。

**不覆盖（归其它主题域）：**

- HTTP transport、重定向、cookie 出入站捕获 → 01 network/session。
- WebView 登录、凭据安全存储（Keychain/Keystore） → 03 login/auth。
- TXT/EPUB 文件解析、文件选择器、沙箱授权 → 04 local book/files。
- 后台任务调度（同步任务何时触发） → 06 ui/background。

**上游事实来源：**

- `ARCHITECTURE.md` §二 模块归属表、§五 阶段5（Core 负责 book source /
  bookshelf / chapter metadata / chapter content cache / reading progress /
  download queue / recent history / Cookie·session / schema migration /
  recovery metadata；平台只传入 `dataDirectory` / `cacheDirectory`）。
- `FEATURE_MATRIX.md`（SQLite schema/迁移/缓存/进度/下载队列/最近历史/
  Cookie·Session 持久化/Recovery·校验·Diff/WebDAV 协议和冲突策略/
  备份·恢复/同步·冲突解决 → Rust Core）。
- `protocol/compatibility.md` §"Runtime Config"（`dataDirectory` /
  `cacheDirectory` 为可选非空字符串）。
- `crates/reader-contract/src/config.rs` `RuntimeConfig`（现行仅两字段）。
- `MIGRATION_MAP.md` 阶段5（当前仅 in-memory cache/progress smoke；
  SQLite 持久化与平台迁移 pending）。

## 2. Capability inventory

| 子能力 | 归属类别 | 当前事实来源 |
|--------|----------|--------------|
| `dataDirectory` / `cacheDirectory` 路径提供 | **Host-owned** | config.rs `RuntimeConfig` |
| 目录创建、权限、生命周期 | **Host-owned** | 本文件立约（现行协议未定义） |
| SQLite 文件物理读写 | **Core-owned**（经 host 提供的路径） | ARCHITECTURE §五 |
| SQLite schema 版本管理 | **Core-owned** | FEATURE_MATRIX |
| SQLite schema 迁移 | **Core-owned** | FEATURE_MATRIX |
| 章节内容缓存（元数据 + 字节） | **Core-owned** | FEATURE_MATRIX |
| 阅读进度持久化 | **Core-owned** | FEATURE_MATRIX |
| 下载队列状态机 | **Core-owned** | FEATURE_MATRIX |
| 下载字节落盘位置 | **Shared-contract** | 本文件立约（现行协议未定义） |
| 最近历史 | **Core-owned** | FEATURE_MATRIX |
| Cookie/Session 持久化（jar 落库） | **Core-owned** | FEATURE_MATRIX（与 01 互补） |
| Recovery / 校验 / Diff 元数据 | **Core-owned** | FEATURE_MATRIX |
| 备份 export（Core → 字节流） | **Core-owned** | FEATURE_MATRIX |
| 备份 import（字节流 → Core） | **Core-owned** | FEATURE_MATRIX |
| 备份文件选择、写入用户可见位置 | **Host-owned** | 本文件立约 |
| WebDAV 协议、冲突策略、diff | **Core-owned** | FEATURE_MATRIX |
| WebDAV HTTP transport | **Shared-contract**（复用 `http.execute`） | 01 network/session |
| WebDAV 凭据（用户名/密码/token） | **Host-owned**（Keychain/Keystore）→ 经 Shared-contract 注入 Core | 本文件立约（与 03 互补） |
| 同步触发调度（何时 push/pull） | **Host-owned**（后台任务） | 06 ui/background |
| 静态加密 at rest | **Out-of-scope** | V1 不交付（见风险） |
| 多账户/多设备多 profile | **Out-of-scope** | V1 不交付 |

## 3. Contracts

现行协议在 storage/sync 域 **完全空白**：`RuntimeConfig` 仅
`{dataDirectory?, cacheDirectory?}`，无 `storage.*` / `sync.*` 方法，
`V1_CAPABILITIES` 不含任何存储/同步能力。下列契约草案均为 **新增**，
属于 protocol schema 变更，须由 protocol schema owner 评估 version bump。

### 3.1 数据目录契约（Host → Core，启动期）

现行 `RuntimeConfig`：

```json
{ "dataDirectory": "/path/to/data", "cacheDirectory": "/path/to/cache" }
```

**Gap A（目录职责与创建责任未定义）：** 协议未规定
- 谁创建这两个目录（host 还是 Core）；
- 目录权限/属主；
- `dataDirectory` 与 `cacheDirectory` 是否允许相同/嵌套；
- Core 是否可在 `dataDirectory` 下自由建子目录。

建议契约（非破坏性，补充语义）：

- **Host-owned**：在 `rc_runtime_create` 前确保两个目录存在且可读写，
  权限符合平台沙箱要求。
- **Core-owned**：在 `dataDirectory` 下自由创建子目录与文件（如
  `data/reader.db`、`data/cookies/`、`data/downloads/`），schema 与命名
  由 Core 管理。
- `cacheDirectory` 可被 host 随时清空（卸载/低空间）；Core 不得在其中
  存放不可重建的数据（进度、jar、下载队列元数据必须落 `dataDirectory`）。
- 两目录不得相同；`dataDirectory` 不得是 `cacheDirectory` 的子目录。

→ 后续 owner: protocol schema（语义补充，可能无需 version bump，仅文档）。

### 3.2 存储命令族（Core-owned，平台 → Core）

建议新增方法（命名空间 `storage.*`）：

| method | 方向 | 用途 |
|--------|------|------|
| `storage.backup.export` | platform → Core | 导出 Core 全量状态为字节流（result: `{ "payloadBase64": "..." }`） |
| `storage.backup.import` | platform → Core | 导入备份字节流（params: `{ "payloadBase64": "..." }`） |
| `storage.cache.clear` | platform → Core | 清空 `cacheDirectory` 内 Core 缓存（不影响 data） |
| `storage.stats` | platform → Core | 返回各表行数/大小，用于诊断 |

**Gap B（无 `storage.*` 方法）：** 现行 `methods` 模块与 schema 均无
`storage.*`。备份/恢复在 FEATURE_MATRIX 标 Core，但无协议入口，host 无法
触发。→ 后续 owner: protocol schema + Core runtime。

### 3.3 同步命令族（Core-owned，platform → Core）

建议新增方法（命名空间 `sync.*`）：

| method | 方向 | 用途 |
|--------|------|------|
| `sync.webdav.configure` | platform → Core | 注入 WebDAV endpoint + 凭据句柄（见 3.4） |
| `sync.webdav.push` | platform → Core | 上传本地变更到 WebDAV |
| `sync.webdav.pull` | platform → Core | 拉取远端变更并合并 |
| `sync.status` | platform → Core | 返回上次同步时间、冲突数、待推送数 |

**Gap C（无 `sync.*` 方法）：** 现行协议无同步入口。WebDAV 协议与冲突
策略归 Core，但 host 无法发起同步。→ 后续 owner: protocol schema +
Core runtime。

### 3.4 WebDAV 凭据注入（Shared-contract，跨 02/03）

WebDAV 凭据属于安全存储（FEATURE_MATRIX: 安全凭据存储 → Platform），
但同步逻辑归 Core。建议：

```json
// sync.webdav.configure params
{
  "endpoint": "https://dav.example.test/reader/",
  "credentialHandle": "webdav-default"
}
```

- `credentialHandle`：host 侧 Keychain/Keystore 的句柄字符串，Core 不持有
  明文凭据。
- 当 Core 需要在 WebDAV HTTP 请求中携带凭据时，通过 `host.request` 向
  host 请求凭据（新 capability `credential.resolve`），host 返回
  `{ "username": "...", "password": "..." }` 或拒绝。
- 凭据仅在单次请求生命周期内由 Core 持有，不落盘、不写日志。

**Gap D（无 `credential.resolve` capability）：** 现行 host bus 仅有
`http.execute` 与 `host.smoke.echo`。安全凭据注入 Core 缺协议通道。
该 capability 跨 02/03，在 03 立约时最终确定。→ 后续 owner:
protocol schema + 03 login/auth。

### 3.5 下载字节落盘（Shared-contract）

下载队列元数据（书/章/进度/状态）归 Core-owned SQLite。但下载的章节
字节体可能较大，落盘位置需协调：

- **Core-owned**：决定"是否需要下载"、"下载到哪一章"、去重、过期清理。
- **Shared-contract**：Core 经 `http.execute` 取回字节后，由 Core 直接
  写入 `dataDirectory/downloads/`（Core-owned 路径），host 不参与字节
  落盘。
- **Host-owned**：仅在用户主动"导出到本地文件"时，由 host 选择目标位置
  （文件选择器，见 04）。

**Gap E（下载字节路径未契约化）：** 现行协议未规定 Core 可在
`dataDirectory` 下写文件。需在 3.1 目录契约中明确 Core 对
`dataDirectory` 的写入权。→ 后续 owner: protocol schema（与 3.1 合并）。

### 3.6 Cookie/Session 持久化边界（与 01 互补）

- **Core-owned**：cookie jar 的 SQLite 表、过期、domain 匹配、持久化。
- **Host-owned**：WebView 读取 cookie（见 03）。
- **Shared-contract**：`session.setCookies`（01 Gap E）把 host 获取的
  cookie 写入 Core jar；Core 在 `dataDirectory/cookies/` 持久化。
- Core jar 不得与平台 HTTP cookie 存储互通（`usePlatformCookieJar=false`）。

## 4. 验收证据要求

> 以下为 *契约成立所需的证据*，不是实现完成的声明。

1. **目录契约证据**：conformance fixture 展示 `dataDirectory` 与
   `cacheDirectory` 相同/嵌套时被 Core 拒绝（`INVALID_PARAMS`），并展示
   Core 在 `dataDirectory` 下创建子目录的成功路径。
2. **schema 迁移证据**：Core 在空库与旧 schema 版本上启动，自动迁移到
   当前 schema，且 `storage.stats` 返回非零行数（用 fixture 数据）。
3. **缓存可清空证据**：`storage.cache.clear` 后，`cacheDirectory` 内 Core
   文件被删除，但 `dataDirectory` 内进度/jar/队列元数据不变。
4. **备份 round-trip 证据**：`storage.backup.export` →
   `storage.backup.import` 在同一 Core 实例上恢复后，`storage.stats`
   返回相同行数。
5. **WebDAV 同步证据**：`sync.webdav.push` / `pull` 经 `http.execute`
   host bus 完成（不绕过 host bus 自建 socket），`sync.status` 反映
   冲突数与时间戳。
6. **凭据隔离证据**：Core 日志/事件中不出现明文 WebDAV 密码；凭据仅经
   `credential.resolve` 临时获取，请求结束后不再持有。
7. **三端一致性证据**：同一备份 payload 在 iOS / Android / Harmony 三端
   import 后，`storage.stats` 与 `book.detail` / `chapter.content` 产出
   相同 canonical DTO。

## 5. Risks

- **目录创建责任真空**：若 host 假设 Core 建目录、Core 假设 host 建目录，
  首次启动会 `INVALID_PARAMS` 或 panic。必须契约化。
- **`cacheDirectory` 误放关键数据**：若 Core 把进度/jar 写入 cache，host
  清缓存后用户进度丢失。必须明确 cache 仅存可重建数据。
- **schema 迁移跨版本断裂**：三端若运行不同 Core commit，schema 版本
  不一致，旧端启动可能破坏新端写入的库。需 schema 版本号 + 向前兼容
  策略。
- **备份格式不可移植**：若备份内嵌 Core 内部二进制布局，跨版本/跨平台
  import 失败。备份格式应为版本化 JSON。
- **WebDAV 凭据泄漏**：若 Core 持久化凭据或写入日志，违反"安全凭据存储
  → Platform"。必须经 `credential.resolve` 临时获取。
- **同步绕过 host bus**：若 Core 为 WebDAV 自建 HTTP socket，绕过
  `http.execute`，则 01 的重定向/cookie/编码契约失效。必须强制 WebDAV
  复用 `http.execute`。
- **静态加密 at rest 缺失**：V1 不做加密，但若任一端自行加 SQLite 加密
  （SQLCipher），会与 Core schema 迁移冲突。显式 Out-of-scope 以防
  split-brain。
- **下载字节与元数据不一致**：若字节落盘失败但队列元数据已更新，会
  产生"已下载但读不到"的幽灵章节。需事务边界。

## 6. Follow-up owners

| 后续工作 | 责任方 |
|----------|--------|
| 目录契约语义补充（Gap A） | protocol schema owner |
| 新增 `storage.*` 方法族（Gap B） | protocol schema + Core runtime |
| 新增 `sync.*` 方法族（Gap C） | protocol schema + Core runtime |
| `credential.resolve` capability（Gap D，跨 02/03） | protocol schema + 03 login/auth |
| 下载字节路径契约（Gap E，并入 3.1） | protocol schema + Core runtime |
| Core SQLite schema + 迁移 + 缓存/进度/队列/jar 实现 | Core storage owner |
| Core WebDAV 协议 + 冲突 + diff 实现 | Core sync owner |
| `session.setCookies` 命令（与 01 Gap E 合并） | protocol schema + 01 network/session |
| iOS `StorageHostAdapter`（目录创建、备份文件选择） | iOS adapter owner |
| Android `StorageHostAdapter`（SAF、备份导出） | Android adapter owner |
| Harmony `StorageHostAdapter`（目录、备份导出） | Harmony adapter owner |
| 同步更新 `protocol/fixtures/conformance/` 与 `reader-contract` 测试 | protocol schema + Core runtime |

---

*本文件立约于 `codex/goal-host-app-contracts`，基线 fb4c3a7。不声明实现完成。*
