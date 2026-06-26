# S5 WebDAV 协议执行 + 独立实体补齐 实施计划

> **For agentic workers:** 本计划闭合章程 S5 阶段两个 gap：(A) WebDAV 协议执行缺失；(B) ReplaceRule/DictRule/TxtTocRule/Bookmark 四类独立实体缺失。TDD 推进，每任务一提交。

**Goal:** 让 Core 产可执行的 WebDAV 请求描述符（Host 执行 HTTP transport），并补齐四类独立可配置实体（对照 Legado + Swift Reader-Core），接入 SqliteStorage 持久化。

**Architecture:**
- `reader-sync`（仅依赖 reader-domain）产自包含 `WebDavRequest`/`WebDavResponse` 描述符 + PROPFIND XML 解析 + 实体 sync 集合。不引入新依赖。
- `reader-runtime`（依赖 reader-sync + reader-contract）把 `WebDavRequest` 桥接为 `HostHttpRequest`，通过 `HostCapability::HttpExecute` 派发；Host 执行真实 HTTP，回 `HostHttpResponse` → `WebDavResponse`。
- `reader-domain` 新增 4 个独立实体（纯数据模型）。
- `reader-storage` SqliteStorage schema V1→V2 迁移加 4 张表 + StorageSnapshot V1→V2 加 4 个集合 + CRUD。

**Tech Stack:** Rust, serde, rusqlite（已就位），无新依赖（XML 用定向手写解析器，匹配 Swift SAX 元素名归一化）。

---

## 章程 §9 五个问题（本计划预答）

1. **兼容目标来自 Legado 哪个路径**：`legado/app/src/main/java/io/legado/app/data/entities/{ReplaceRule,DictRule,TxtTocRule,Bookmark}.kt` 四实体；WebDAV 协议语义对照 Legado 备份目录约定（`reader-core/backups/{id}.json` + MKCOL 建目录）。
2. **迁移资产来自 Reader-Core 哪个路径**：`URLSessionWebDAVAdapter.swift`（PROPFIND/GET/PUT/DELETE + multistatus 解析）、`ReaderCoreWebDAVSyncRuntime.swift`（LWW per-(book,chapter,device) + 文件布局）、`DictRule.swift`、`ReplaceRuleEngine.swift`（ReaderCoreManagedReplaceRule + scope 过滤）。TxtTocRule Swift 缺，对照 Legado 新建。
3. **Rust 改动落在哪**：`crates/reader-domain`（4 实体）、`crates/reader-sync`（WebDAV 描述符 + 协议解析 + sync.backup 真实 descriptor + SyncCollection）、`crates/reader-storage`（schema V2 + Snapshot V2 + CRUD）、`crates/reader-runtime`（WebDavRequest→HostHttpRequest 桥接）。
4. **是否改变三端 host adapter 责任边界**：不改变。Core 仍只产描述符，Host 仍执行 HTTP transport。WebDavRequest 复用现有 `HostCapability::HttpExecute`，无需新 capability。
5. **证据层级**：本轮为 crate test（`cargo test -p reader-sync -p reader-storage -p reader-domain`）。非 wrapper smoke / App/device proof / corpus benchmark。

---

## 文件结构

### 新建文件
- `crates/reader-sync/src/webdav_protocol.rs` — WebDAV 描述符 + PROPFIND XML 解析（新模块）
- `crates/reader-sync/src/webdav_client.rs` — 高层 WebDAV 操作（list/download/upload/delete/mkdir/test，产 WebDavRequest）
- `crates/reader-sync/src/webdav_backup.rs` — sync.backup 真实 descriptor 序列 + 保留策略
- `crates/reader-sync/src/webdav_conflict.rs` — LWW + journal 冲突解决

### 修改文件
- `crates/reader-domain/src/lib.rs` — 新增 TxtTocRule / Bookmark / ReplaceRule / DictRule 4 实体 + ReplaceRule scope 过滤
- `crates/reader-sync/src/lib.rs` — `pub mod webdav_*` 导出 + SyncCollection 加 4 变体
- `crates/reader-storage/src/lib.rs` — StorageSnapshot V2 + 4 集合 + InMemoryStorage CRUD
- `crates/reader-storage/src/sqlite_backend.rs` — SCHEMA_V2_DDL + migrate v1→v2 + SqliteStorage CRUD
- `crates/reader-runtime/src/remote.rs` — sync.backup 桥接 WebDavRequest→HostHttpRequest 派发

### 测试文件
- `crates/reader-sync/tests/webdav_protocol.rs`
- `crates/reader-sync/tests/webdav_backup_descriptors.rs`
- `crates/reader-sync/tests/webdav_conflict.rs`
- `crates/reader-domain/tests/independent_entities.rs`
- `crates/reader-storage/tests/independent_entities_crud.rs`

---

## Task A1: WebDAV 请求/响应描述符 + PROPFIND XML 解析

**Files:**
- Create: `crates/reader-sync/src/webdav_protocol.rs`
- Test: `crates/reader-sync/tests/webdav_protocol.rs`

**关键类型**（对照 Swift `URLSessionWebDAVAdapter` + `WebDAVMultistatusParser`）：

```rust
pub enum WebDavMethod { Propfind, Get, Put, Delete, Mkcol, Move, Copy }

pub struct WebDavRequest {
    pub method: WebDavMethod,
    pub path: String,            // 相对 base_url 的路径
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub depth: Option<u8>,       // PROPFIND 专用
    pub accepted_status_codes: Vec<u16>,
}

pub struct WebDavResponse {
    pub status: u16,
    pub headers: serde_json::Value,
    pub body: Vec<u8>,
    pub final_url: Option<String>,
}

pub struct WebDavResource {  // PROPFIND 解析结果
    pub href: String,
    pub content_length: Option<i64>,
    pub last_modified: Option<String>,
    pub etag: Option<String>,
    pub is_collection: bool,
}
```

**PROPFIND 请求体**（匹配 Swift 第 90-100 行）：
```rust
pub fn propfind_request_body() -> &'static str {
    r#"<?xml version="1.0" encoding="utf-8"?>
<propfind xmlns="DAV:">
  <prop>
    <getcontentlength/>
    <getlastmodified/>
    <getetag/>
    <resourcetype/>
  </prop>
</propfind>"#
}
```

**multistatus 解析**（手写状态机，元素名归一化匹配 Swift `normalizedElementName`，支持 `D:`/`d:`/无前缀）：
- `parse_multistatus(body: &[u8]) -> Result<Vec<WebDavResource>, WebDavProtocolError>`
- 解析 `<response>`/`<href>`/`<getcontentlength>`/`<getlastmodified>`/`<getetag>`/`<collection>`

**步骤：**
- [ ] 写失败测试：propfind_request_body 返回期望 XML；parse_multistatus 解析单资源/多资源/带命名空间前缀/空集合
- [ ] 运行测试确认失败
- [ ] 实现 WebDavMethod/WebDavRequest/WebDavResponse/WebDavResource + propfind_request_body + parse_multistatus
- [ ] 测试通过
- [ ] 提交 `feat(reader-sync): add WebDAV request descriptor and PROPFIND multistatus parser`

---

## Task A2: WebDAV 高层客户端操作（产 WebDavRequest）

**Files:**
- Create: `crates/reader-sync/src/webdav_client.rs`
- Test: `crates/reader-sync/tests/webdav_protocol.rs`（追加）

**函数**（对照 Swift adapter 方法，每个产 WebDavRequest，不发 HTTP）：
```rust
pub fn list_directory(path: &str) -> WebDavRequest           // PROPFIND Depth:1
pub fn download_file(path: &str) -> WebDavRequest             // GET
pub fn upload_file(path: &str, body: Vec<u8>) -> WebDavRequest  // PUT octet-stream
pub fn upload_json(path: &str, json: &str) -> WebDavRequest    // PUT application/json
pub fn delete_file(path: &str) -> WebDavRequest              // DELETE
pub fn make_collection(path: &str) -> WebDavRequest          // MKCOL（Swift 缺，对照 Legado 补）
pub fn connection_test(path: &str) -> WebDavRequest          // PROPFIND Depth:0
```

**步骤：**
- [ ] 写失败测试：每个函数产期望 method/path/headers/body/depth/accepted_status_codes
- [ ] 实现 7 个函数
- [ ] 测试通过
- [ ] 提交 `feat(reader-sync): add WebDAV high-level client operations`

---

## Task A3: sync.backup 产真实备份 descriptor 序列

**Files:**
- Create: `crates/reader-sync/src/webdav_backup.rs`
- Test: `crates/reader-sync/tests/webdav_backup_descriptors.rs`

**函数**（对照 Swift `ReaderCoreWebDAVSyncRuntime` 文件布局 `reader-core/backups/{id}.json`）：
```rust
pub fn build_backup_upload_descriptors(
    backup_dir: &str, backup_id: &str, package_json: &str,
) -> Vec<WebDavRequest>  // [MKCOL backup_dir（容错）, PUT {backup_dir}/{backup_id}.json]

pub fn build_backup_list_descriptor(backup_dir: &str) -> WebDavRequest  // PROPFIND backup_dir

pub fn build_backup_download_descriptor(backup_dir: &str, backup_id: &str) -> WebDavRequest  // GET

pub fn build_backup_retention_delete_descriptors(
    backup_dir: &str, expired_ids: &[String],
) -> Vec<WebDavRequest>  // 每个过期 id 一个 DELETE
```

**步骤：**
- [ ] 写失败测试：路径拼接 `reader-core/backups/{id}.json`；MKCOL 在 PUT 前；retention 产 N 个 DELETE
- [ ] 实现 4 个函数
- [ ] 测试通过
- [ ] 提交 `feat(reader-sync): produce real WebDAV backup descriptors`

---

## Task A4: 冲突解决 LWW + journal

**Files:**
- Create: `crates/reader-sync/src/webdav_conflict.rs`
- Test: `crates/reader-sync/tests/webdav_conflict.rs`

**LWW**（对照 Swift `resolveConflicts` per-(book,chapter,device)，复用现有 `ProgressCloudSyncRecord`）：
```rust
pub fn resolve_progress_conflicts_lww(
    local: Vec<ProgressCloudSyncRecord>,
    remote: Vec<ProgressCloudSyncRecord>,
) -> Vec<ProgressCloudSyncRecord>
// 合并 + 按 updated_at 降序 + (book_id, chapter_index, device_id) 去重保留最新
```

**Journal**（新增，超出 Swift，补齐冲突审计）：
```rust
pub struct SyncJournalEntry {
    pub seq: u64,
    pub timestamp: i64,
    pub operation: SyncJournalOperation,  // Push/Pull/Backup/Restore
    pub collection: SyncCollection,
    pub record_key: Option<SyncRecordKey>,
    pub outcome: SyncJournalEntryStatus,  // Success/Skipped/Conflict/Failed
    pub details: Option<String>,
}

pub fn append_journal_entry(journal: &mut Vec<SyncJournalEntry>, entry: SyncJournalEntry)
// seq 单调递增；时间戳不回退

pub fn journal_entries_since(journal: &[SyncJournalEntry], since: i64) -> Vec<&SyncJournalEntry>
```

**步骤：**
- [ ] 写失败测试：LWW 选最新 updated_at；跨设备同章节保留最新；journal seq 单调；journal_since 过滤
- [ ] 实现 LWW + journal
- [ ] 测试通过
- [ ] 提交 `feat(reader-sync): formalize LWW conflict resolution and sync journal`

---

## Task B1: TxtTocRule 实体（reader-domain）

**Files:**
- Modify: `crates/reader-domain/src/lib.rs`
- Test: `crates/reader-domain/tests/independent_entities.rs`

**对照 Legado `TxtTocRule.kt`**（Swift 缺，对照 Legado 新建）：
```rust
pub struct TxtTocRule {
    pub id: i64,                              // PK, 默认 now millis
    #[serde(default)] pub name: String,
    #[serde(default)] pub rule: String,       // 正则
    #[serde(default, skip_serializing_if = "Option::is_none")] pub example: Option<String>,
    #[serde(default = "default_serial_number")] pub serial_number: i32,  // 默认 -1
    #[serde(default = "default_true")] pub enable: bool,
}
```

**步骤：**
- [ ] 写失败测试：serde roundtrip；默认值（serial_number=-1, enable=true）；deny_unknown_fields
- [ ] 实现 TxtTocRule
- [ ] 测试通过
- [ ] 提交 `feat(reader-domain): add TxtTocRule entity against Legado`

---

## Task B2: Bookmark 实体（reader-domain）

**Files:**
- Modify: `crates/reader-domain/src/lib.rs`
- Test: `crates/reader-domain/tests/independent_entities.rs`

**对照 Legado `Bookmark.kt`**（Swift 仅有轻量 draft，对齐 Legado 字段）：
```rust
pub struct Bookmark {
    pub time: i64,                            // PK, 创建时间戳
    #[serde(default)] pub book_name: String,
    #[serde(default)] pub book_author: String,
    #[serde(default)] pub chapter_index: i32,
    #[serde(default)] pub chapter_pos: i32,
    #[serde(default)] pub chapter_name: String,
    #[serde(default)] pub book_text: String,
    #[serde(default)] pub content: String,    // 用户批注
}
```

**步骤：**
- [ ] 写失败测试：serde roundtrip；默认值；deny_unknown_fields
- [ ] 实现 Bookmark
- [ ] 测试通过
- [ ] 提交 `feat(reader-domain): add Bookmark entity against Legado`

---

## Task B3: ReplaceRule 独立实体 + scope 过滤（reader-domain）

**Files:**
- Modify: `crates/reader-domain/src/lib.rs`
- Test: `crates/reader-domain/tests/independent_entities.rs`

**对照 Swift `ReaderCoreManagedReplaceRule` + Legado `ReplaceRule.kt`**（合并字段，保留 Legado `is_regex`/`order`/`group` + Swift scope 过滤语义）：
```rust
pub struct ReplaceRule {
    pub id: i64,                              // PK
    #[serde(default)] pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub group: Option<String>,
    #[serde(default)] pub pattern: String,
    #[serde(default)] pub replacement: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub scope: Option<String>,
    #[serde(default)] pub scope_title: bool,        // 默认 false
    #[serde(default = "default_true")] pub scope_content: bool,  // 默认 true
    #[serde(default, skip_serializing_if = "Option::is_none")] pub exclude_scope: Option<String>,
    #[serde(default = "default_true")] pub is_enabled: bool,
    #[serde(default = "default_true")] pub is_regex: bool,
    #[serde(default = "default_timeout_ms")] pub timeout_millisecond: i64,  // 默认 3000
    #[serde(default)] pub order: i32,               // DB 列名 sortOrder
}

pub struct ReplaceRuleEvaluationContext {
    pub book_title: String,
    pub source_name: String,
    pub source_url: String,
}

// scope 过滤（对照 Swift matches_target/matches_include_scope/matches_exclude_scope）
pub fn replace_rule_matches_target(rule: &ReplaceRule, target: ReplaceRuleTarget) -> bool
pub fn replace_rule_matches_scope(rule: &ReplaceRule, ctx: &ReplaceRuleEvaluationContext) -> bool
pub fn scope_tokens(scope: Option<&str>) -> Vec<String>  // 按 , ; | 切分，小写
```

**步骤：**
- [ ] 写失败测试：scope token 切分；include ANY 语义；exclude ANY 语义；target 过滤；空 scope 匹配全部
- [ ] 实现 ReplaceRule + scope 过滤函数
- [ ] 测试通过
- [ ] 提交 `feat(reader-domain): add standalone ReplaceRule entity with scope filtering`

---

## Task B4: DictRule 实体（reader-domain）

**Files:**
- Modify: `crates/reader-domain/src/lib.rs`
- Test: `crates/reader-domain/tests/independent_entities.rs`

**对照 Swift `DictRule.swift` + Legado `DictRule.kt`**：
```rust
pub struct DictRule {
    pub name: String,                         // PK
    #[serde(default)] pub url_rule: String,
    #[serde(default)] pub show_rule: String,
    #[serde(default = "default_true")] pub enabled: bool,
    #[serde(default)] pub sort_number: i32,
}
```

**步骤：**
- [ ] 写失败测试：serde roundtrip；默认值
- [ ] 实现 DictRule
- [ ] 测试通过
- [ ] 提交 `feat(reader-domain): add DictRule entity against Swift/Legado`

---

## Task B5: SqliteStorage schema V2 迁移（4 张新表）

**Files:**
- Modify: `crates/reader-storage/src/sqlite_backend.rs`
- Test: `crates/reader-storage/tests/independent_entities_crud.rs`

**SCHEMA_V2_DDL**（对照 Legado 表名 + 列名）：
```sql
CREATE TABLE IF NOT EXISTS replace_rules (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL DEFAULT '',
    "group" TEXT,
    pattern TEXT NOT NULL DEFAULT '',
    replacement TEXT NOT NULL DEFAULT '',
    scope TEXT,
    scope_title INTEGER NOT NULL DEFAULT 0,
    scope_content INTEGER NOT NULL DEFAULT 1,
    exclude_scope TEXT,
    is_enabled INTEGER NOT NULL DEFAULT 1,
    is_regex INTEGER NOT NULL DEFAULT 1,
    timeout_millisecond INTEGER NOT NULL DEFAULT 3000,
    sort_order INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_replace_rules_enabled ON replace_rules(is_enabled, sort_order);

CREATE TABLE IF NOT EXISTS dict_rules (
    name TEXT PRIMARY KEY,
    url_rule TEXT NOT NULL DEFAULT '',
    show_rule TEXT NOT NULL DEFAULT '',
    enabled INTEGER NOT NULL DEFAULT 1,
    sort_number INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS txt_toc_rules (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL DEFAULT '',
    rule TEXT NOT NULL DEFAULT '',
    example TEXT,
    serial_number INTEGER NOT NULL DEFAULT -1,
    enable INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX IF NOT EXISTS idx_txt_toc_rules_enable ON txt_toc_rules(enable, serial_number);

CREATE TABLE IF NOT EXISTS bookmarks (
    time INTEGER PRIMARY KEY,
    book_name TEXT NOT NULL DEFAULT '',
    book_author TEXT NOT NULL DEFAULT '',
    chapter_index INTEGER NOT NULL DEFAULT 0,
    chapter_pos INTEGER NOT NULL DEFAULT 0,
    chapter_name TEXT NOT NULL DEFAULT '',
    book_text TEXT NOT NULL DEFAULT '',
    content TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_bookmarks_book ON bookmarks(book_name, book_author);
```

**步骤：**
- [ ] 写失败测试：open_in_memory 后 4 张表存在；user_version=2；从 v1 数据库迁移到 v2 不丢数据
- [ ] bump `SQLITE_SCHEMA_VERSION = 2`，加 SCHEMA_V2_DDL，migrate 加 `if current < 2 { ... }`
- [ ] 测试通过
- [ ] 提交 `feat(reader-storage): add schema V2 migration for 4 independent entities`

---

## Task B6: StorageSnapshot V2 + 4 集合

**Files:**
- Modify: `crates/reader-storage/src/lib.rs`
- Test: `crates/reader-storage/tests/independent_entities_crud.rs`

**变更：**
- `STORAGE_SNAPSHOT_SCHEMA_VERSION = 2`
- `StorageSnapshot` 加 4 字段：`replace_rules: Vec<ReplaceRule>`, `dict_rules: Vec<DictRule>`, `txt_toc_rules: Vec<TxtTocRule>`, `bookmarks: Vec<Bookmark>`
- `migrate_storage_snapshot` 1→2 加空 vec
- `StorageSnapshot::empty` 初始化 4 空 vec
- `replace_with_snapshot` 清空 4 表 + 重灌

**步骤：**
- [ ] 写失败测试：snapshot 含 4 集合 roundtrip；v1 snapshot 迁移到 v2 加空 vec；canonical hash 稳定
- [ ] 实现 V2 snapshot + 迁移
- [ ] 测试通过
- [ ] 提交 `feat(reader-storage): extend StorageSnapshot to V2 with 4 independent entities`

---

## Task B7: SqliteStorage + InMemoryStorage CRUD（4 实体）

**Files:**
- Modify: `crates/reader-storage/src/lib.rs`, `crates/reader-storage/src/sqlite_backend.rs`
- Test: `crates/reader-storage/tests/independent_entities_crud.rs`

**CRUD**（每实体 put/get/list/delete + 定向查询）：
- `put_replace_rule` / `get_replace_rule` / `list_replace_rules` / `delete_replace_rule` / `find_enabled_replace_rules_by_scope(name, origin, target)`
- `put_dict_rule` / `get_dict_rule` / `list_dict_rules` / `delete_dict_rule`
- `put_txt_toc_rule` / `get_txt_toc_rule` / `list_txt_toc_rules` / `list_enabled_txt_toc_rules()` / `delete_txt_toc_rule`
- `put_bookmark` / `list_bookmarks_by_book(name, author)` / `search_bookmarks(key)` / `delete_bookmark`

**步骤：**
- [ ] 写失败测试：每实体 CRUD roundtrip；ReplaceRule scope 查询；Bookmark by book + search；TxtTocRule enable 过滤 + serial_number 排序
- [ ] 实现 InMemoryStorage + SqliteStorage 并行 CRUD
- [ ] 测试通过
- [ ] 提交 `feat(reader-storage): add CRUD for 4 independent entities`

---

## Task B8: SyncCollection 加 4 变体 + 导出 webdav 模块

**Files:**
- Modify: `crates/reader-sync/src/lib.rs`
- Test: `crates/reader-sync/tests/webdav_protocol.rs`（追加）

**变更：**
- `SyncCollection` 加 `ReplaceRule`, `DictRule`, `TxtTocRule`, `Bookmark` 变体 + as_str()
- `pub mod webdav_protocol; pub mod webdav_client; pub mod webdav_backup; pub mod webdav_conflict;`

**步骤：**
- [ ] 写失败测试：4 新变体 as_str 正确；roundtrip
- [ ] 实现 + 导出
- [ ] 测试通过
- [ ] 提交 `feat(reader-sync): add 4 independent entity SyncCollection variants`

---

## Task A5: reader-runtime 桥接 WebDavRequest → HostHttpRequest

**Files:**
- Modify: `crates/reader-runtime/src/remote.rs`
- Test: `crates/reader-runtime` 现有测试 + 新增桥接测试

**桥接函数**（runtime 依赖 reader-sync + reader-contract，是唯一能同时访问两者的层）：
```rust
fn webdav_request_to_host_http(req: &WebDavRequest, base_url: &str, auth: Option<&str>) -> HostHttpRequest
fn host_http_response_to_webdav(resp: &HostHttpResponse) -> WebDavResponse
```
- method 字符串映射：Propfind→"PROPFIND", Mkcol→"MKCOL", 等
- headers 注入 Depth / Content-Type / Authorization
- sync.backup 改为产 WebDavRequest 序列 → 桥接 → HttpExecute 派发

**步骤：**
- [ ] 写失败测试：桥接 roundtrip；PROPFIND 带 Depth 头；MKCOL 无 body
- [ ] 实现桥接 + sync.backup 派发路径（保留 plan 路径作 fallback）
- [ ] 测试通过
- [ ] 提交 `feat(reader-runtime): bridge WebDavRequest to HostHttpRequest dispatch`

---

## Task C: 验证

- [ ] `cargo test -p reader-domain` 通过
- [ ] `cargo test -p reader-storage`（含 sqlite feature）通过
- [ ] `cargo test -p reader-sync` 通过
- [ ] `cargo test -p reader-runtime` 通过
- [ ] `cargo fmt --all --check` 通过
- [ ] `cargo clippy -p reader-sync -p reader-storage -p reader-domain -- -D warnings` 通过

---

## 自检

**Spec coverage:**
- A.1 WebDAV descriptor (PROPFIND/PUT/GET/DELETE/MKCOL) → Task A1+A2 ✓
- A.2 sync.backup 产真实备份 descriptor → Task A3 ✓
- A.3 冲突解决 LWW + journal → Task A4 ✓
- B.1 TxtTocRule → Task B1 ✓
- B.2 Bookmark → Task B2 ✓
- B.3 ReplaceRule 独立管理 + scope → Task B3 ✓
- B.4 SqliteStorage 持久化 → Task B5+B6+B7 ✓
- (DictRule 补齐 → Task B4，因迁移源 DictRule.swift 已提供且为"四类"之一)
- Core 产 descriptor, Host 执行 HTTP → Task A5 桥接 ✓

**Placeholder scan:** 无 TBD/TODO；每任务有具体类型定义 + 测试。

**Type consistency:** `WebDavRequest`/`WebDavResponse`/`WebDavResource` 跨 A1-A5 一致；4 实体跨 B1-B7 一致；`SyncCollection` 4 新变体跨 B8/A4 一致。
