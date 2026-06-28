# 能力清单代码级审计修正

日期：2026-06-29
审计方法：两个独立 explorer agent 对 `crates/` 目录做 grep + 代码路径验证
前置文档：`docs/LEGADO_CAPABILITY_INVENTORY.md`（97 项能力清单）

本文是对清单中 12 项 "🔴 0 代码" 标记的代码级交叉验证结果。
**结论：7/12 项标记错误，清单低估了已实现能力。**

---

## 修正矩阵

| # | 能力 | 清单原标记 | 实际状态 | 证据 | 修正后 |
|---|------|-----------|---------|------|--------|
| 1 | source.explore dispatch | 🔴 注释禁用/死代码 | dispatch 活跃，handler 可达，有11个测试 | `remote.rs:466-468` 活跃 dispatch; `explore_kinds.rs` 11 tests | 🟢 |
| 2 | importScript | 🔴 0 代码 | HostDescriptor 已路由+测试 | `reader-js/src/lib.rs:231,8040`; `host_routing_residual.rs:151` | 🟢 |
| 3 | queryTTF/replaceFont | 🔴 0 代码 | HostDescriptor 已路由+测试 | `reader-js/src/lib.rs:354,362,8194`; `host_routing_s3_closure.rs:465,516` | 🟢 |
| 4 | RssStar | 🔴 0 代码 | 已实现+测试 | `reader-rss/src/lib.rs:665,2125`; tests at `:8175,8397` | 🟢 |
| 5 | ReviewRule(段评) | 🔴 0 代码 | struct 存在，无 dispatch | `reader-domain/src/lib.rs:543` struct; 0 dispatch in remote.rs | 🟠 |
| 6 | Umd | 🔴 0 代码 | 检测+MIME已实现，parser 延后 | `reader-local-book/src/lib.rs:46,3189,4823`; test `:8974` | 🟡 |
| 7 | RssReadRecord | 🔴 0 代码 | 读状态已跟踪，无独立实体 | `reader-rss/src/lib.rs:919,2125` entry state map | 🟠 |
| 8 | 规则缓存 | 🔴 0 代码 | regex cache 存在，无 rule-string cache | `reader-rule/src/lib.rs:24,606` cached_regex | 🟡(misleading) |
| 9 | multipart | 🔴 0 代码 | 确认缺失 | 0 matches | 🔴(accurate) |
| 10 | GlideUrl | 🔴 0 代码 | 确认缺失（Host 侧能力） | 0 matches | 🔴(accurate) |
| 11 | CheckSource | 🔴 0 代码 | 确认缺失 | 0 matches | 🔴(accurate) |
| 12 | 书源调试 Debug | 🔴 0 代码 | 确认缺失 | 0 matches | 🔴(accurate) |

---

## 关键发现

### 清单低估了 4 项已实现+测试的能力

1. **explore dispatch 是活的**（非注释禁用）— `SOURCE_EXPLORE` 和 `SOURCE_EXPLORE_KINDS` 都在 `remote.rs:466-468` 活跃 dispatch，有 continuation 链路和 11 个测试
2. **importScript 已路由** — `java.importScript(path)` → `HostDescriptor::ImportScript`，有 mock host 测试
3. **queryTTF/replaceFont 已路由** — `java.queryTTF`/`java.replaceFont` → HostDescriptor，有端到端字体反混淆测试
4. **RssStar 已实现** — `set_entry_starred()` + `starred` flag + 3 个测试

### 清单低估了 3 项部分实现的能力

5. **ReviewRule struct 存在** — 域模型已定义 5 个字段，但无 dispatch/handler
6. **Umd 检测已实现** — 格式检测+MIME映射+测试，parser 有意延后（Legado 也委托第三方）
7. **RssReadRecord 读状态已跟踪** — 通过 entry state map 实现，无独立实体

### 确认缺失的 4 项（清单准确）

8. **multipart 文件上传** — 0 代码（WebDAV PUT 不是 multipart）
9. **GlideUrl** — 0 代码（Host 侧图片加载，Core 不应有）
10. **CheckSource 书源校验** — 0 代码
11. **书源调试 Debug** — 0 代码

### 规则缓存（nuanced）

12. **splitSourceRuleCacheString** — 有 regex cache (`cached_regex` module)，但无 Legado 的 rule-string cache。清单 "0 代码" 不准确但方向正确。

---

## 对能力统计的影响

修正前（清单原统计）：
- 🔴 未实现：32/109 = 29%

修正后：
- 🔴 确认缺失：26/109 = 24%（减少 6 项：explore/importScript/queryTTF/RssStar → 🟢，Umd → 🟡，ReviewRule/RssReadRecord → 🟠）
- 🟢 已实现+测试：增加 4 项（explore/importScript/queryTTF/RssStar）
- 🟡/🟠 部分实现：增加 3 项（Umd/ReviewRule/RssReadRecord）

---

## 批量测试状态（2026-06-29 00:30 CST）

### v4 基线（459 源全量）

| 级别 | 通过 | 通过率 |
|------|------|--------|
| L1-import | 459/459 | 100% |
| L2-search | 75/459 | 16.3% |
| L3-detail | 50/459 | 10.9% |
| L4-toc | 16/459 | 3.5% |
| L5-content | 5/459 | 1.1% |
| fully_passed | 5/459 | 1.1% |

### v5 前 100 源（本轮修复后）

| 级别 | 通过 | 通过率 |
|------|------|--------|
| L1-import | 100/100 | 100% |
| L2-search | 12/100 | 12% |
| L3-detail | 6/100 | 6% |
| L4-toc | 4/100 | 4% |
| L5-content | 2/100 | 2% |
| fully_passed | 2/100 | 2% |

### v5 全量 459 源

- 测试进行中（concurrency=1，预计 15-20 分钟）
- 结果将写入 `reports/tooling/corpus-batch-v5-full.json`

### 失败原因分布（v5-100）

| 原因 | 数量 | 占比 |
|------|------|------|
| no_search_results | 38 | 38% |
| js_unsupported | 24 | 24% |
| http_error | 23 | 23% |
| core_error | 9 | 9% |
| content_too_short | 2 | 2% |
| no_toc_entries | 2 | 2% |

---

## 真实阻塞点（基于代码+批量测试）

### P0 — 阻断最大量书源

1. **no_search_results (38%)** — HTTP 成功但解析返回空
   - 根因：bookList 规则不匹配 / has_js 误分类 / 规则补全不完整
   - 影响：~174/459 源

2. **js_unsupported (24%)** — JS 沙箱执行失败
   - 根因：searchUrl 中的 `@js:` / `<js>` 执行失败
   - 影响：~110/459 源（v4 是 96，v5-100 是 24）

3. **http_error (23%)** — HTTP 请求失败
   - 根因：IDN 域名解析失败 / User-Agent 缺失 / anti-crawl
   - 影响：~106/459 源

### P1 — 链路断裂

4. **core_error (9%)** — Core 内部错误
   - 根因：JSONPath 变体 / 规则引擎内部错误
   - 影响：~41/459 源

5. **content_too_short (2%)** — 正文提取不足 50 字符
   - 影响：~9/459 源

### P2 — 能力缺失（确认 0 代码）

6. **CheckSource** — 书源校验，影响用户导入体验
7. **书源调试 Debug** — 影响开发调试效率
8. **multipart 文件上传** — 影响少量需要上传的源

