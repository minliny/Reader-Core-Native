# 项目状态快照

扫描日期：2026-06-29（v5 — 能力清单修正 + 100源批量验证）
本文为时间点快照，不作为永久事实。后续工作必须以实际代码验证为准。
前置快照：docs/STATUS_SNAPSHOT_2026-06-28.md v4（P0 修复后批量验证）

---

## 0. 当前状态摘要

| 维度 | 状态 |
|------|------|
| **编译** | ✅ `cargo build -p reader-cli --release` 0 errors |
| **全量 459 源批量** | v5 进行中（100源样本：L2=12%, fully_passed=2） |
| **Release blockers** | ~52 active |
| **Legado 能力清单** | 修正 7 项错误标记（explorer agent 代码级审计） |

---

## 1. 能力清单修正（2026-06-29 真实代码审计）

两个 explorer agent 对 `docs/LEGADO_CAPABILITY_INVENTORY.md` 中标记为 🔴 的 12 项能力
做了代码级交叉验证，发现 **7 项标记错误**：

| # | 能力 | 原标记 | 实际状态 | 证据 |
|---|------|--------|----------|------|
| 1 | source.explore dispatch | 🔴 死代码 | 🟢 已实现+dispatched+tested | remote.rs:466-468 活跃 dispatch；explore_kinds.rs 11 tests |
| 2 | importScript | 🔴 0代码 | 🟢 已路由+tested | reader-js ImportScript descriptor + host_routing_residual.rs:151 |
| 3 | queryTTF/replaceFont | 🔴 0代码 | 🟢 已路由+tested | reader-js QueryTTF/ReplaceFont + host_routing_s3_closure.rs:465,516 |
| 4 | RssStar | 🔴 0代码 | 🟢 已实现+tested | reader-rss starred bool + set_entry_starred + 8175/8397/8427 tests |
| 5 | ReviewRule/段评 | 🔴 0代码 | 🟠 struct存在,无dispatch | reader-domain ReviewRule struct, remote.rs 0 matches |
| 6 | Umd | 🔴 0代码 | 🟡 检测已实现,parser延后 | reader-local-book Umd variant + detection tests, parser deferred (Legado也委托第三方) |
| 7 | RssReadRecord | 🔴 0代码 | 🟠 读状态已跟踪,无独立实体 | reader-rss entry state map 含 read tracking |

**确认准确**的 5 项：multipart upload(🔴), GlideUrl(🔴), CheckSource(🔴), Debug(🔴), rule cache(🔴)

---

## 2. batch v5 部分（100源样本）

| 级别 | v4全量459 | v5-100样本 | 备注 |
|------|-----------|-----------|------|
| L1-import | 100% | 100% | — |
| L2-search | 16.3% | 12.0% | 样本偏差（前100源质量较低） |
| L3-detail | 10.9% | 6.0% | L2传导 |
| L4-toc | 3.5% | 4.0% | — |
| L5-content | 1.1% | 2.0% | — |
| fully_passed | 5(1.1%) | 2(2.0%) | — |

失败原因分布（100源样本）：
- no_search_results: 38（最大失败原因 — bookList 规则不匹配）
- js_unsupported: 24（JS 沙箱中 java.* 方法未全通）
- http_error: 23（IDN 中文域名 + 站点死亡 + TLS）
- core_error: 9（JSONPath 变体 + 规则引擎内部错误）
- content_too_short: 2, no_toc_entries: 2

---

## 3. 关键阻塞点（真实代码验证）

### P0 — 阻断真实书源跑通
1. **no_search_results (38%)** — bookList 规则解析返回空。需逐源调试规则匹配
2. **js_unsupported (24%)** — JS 沙箱中 java.get/post/ajax 等 host callback 未端到端通
3. **http_error (23%)** — 含 IDN 中文域名问题（ureq/url crate 限制）
4. **core_error (9%)** — JSONPath 变体 + 规则引擎内部错误

### P1 — 已识别但未修
5. **has_js misclassification** — corpus-manager.py detect_rule_forms 需检查 `"@js:" in rule_str`
6. **Chinese domain IDN** — 35 源因 IdnaError 失败
7. **全量459源批量测试进程中断** — 某些源导致 panic，需定位

---

## 4. 全量459源批量测试状态

- v5 全量测试多次尝试：concurrency=3 时约6个源后进程退出
- concurrency=1 时 100源/200源可正常完成
- **推测**：concurrency>1 时某些源的并发处理触发 panic（可能是 JS 沙箱非线程安全）
- **临时方案**：用 concurrency=1 跑全量（慢但稳）

## 测试工具链优化（2026-06-29 v6）

### 已修复的问题

1. **单源详细日志（`--verbose`/`-v`）**：`test_source` 现在输出每步的详细诊断
   - L1-L5 每步的 HTTP 请求 URL、状态码、响应大小
   - Core 返回的书籍数、章节数
   - 失败原因和错误详情
   - 用法：`reader-cli --test-source <path> --keyword "斗破苍穹" --verbose`

2. **panic 隔离**：`test_corpus` 中每个源在 `catch_unwind` 中运行
   - 单个源 panic 不会终止整个批量测试
   - panic 信息记录到结果中

3. **中间结果保存（`--save-interval N`）**：每 N 个源写一次 checkpoint
   - 即使进程被杀，也有部分结果
   - 默认值 10

4. **Runtime::new panic 修复**：`Runtime::new` → `Runtime::new_with_config`
   - worker thread 创建失败不再 panic 终止进程
   - 返回 `runtime_spawn_failed` 错误结果

5. **HTTP method allowlist**：只允许 GET/POST/PUT/DELETE/HEAD/PATCH
   - 防止 CRLF 注入

6. **UTF-8 安全 truncate**：`&s[..max]` → `s.chars().take(max).collect()`
   - 防止中文书源名 panic

### 已知遗留问题

1. **Runtime drop 阻塞**：worker thread 在 JS host callback 中阻塞时，
   `Drop` 的 `join()` 会阻塞直到超时。导致批量测试在270/459处卡住。
   - 修复方向：在 `Drop` 中加超时 join 或 force-kill worker thread
2. **concurrency=1 串行**：太慢，459源需要20+分钟
   - 未来需要实现并发（每源独立 Runtime，用线程池）

### v6 批量测试结果（270/459 源，部分运行）

| 级别 | 通过 | 通过率 |
|------|------|--------|
| L1-import | 270/270 | 100% |
| L2-search | 41/270 | 15.2% |
| L3-detail | 27/270 | 10.0% |
| L4-toc | 11/270 | 4.1% |
| L5-content | 7/270 | 2.6% |
| fully_passed | 7/270 | 2.6% |

失败原因分布：
- no_search_results: 103 (38%) — HTTP 成功但规则解析返回空
- http_error: 61 (23%) — TLS/网络/IDN 问题
- js_unsupported: 54 (20%) — JS 沙箱能力不足
- core_error: 29 (11%) — CSS 选择器/JSONPath 解析错误
- no_toc_entries: 9 (3%)
- content_too_short: 3, parse_error: 3, no_chapter_url: 1

### v4 vs v6 对比

| 指标 | v4 (459源) | v6 (270源) | 变化 |
|------|-----------|-----------|------|
| L2-search | 75 (16.3%) | 41 (15.2%) | ~持平(不同源集) |
| L5-content | 5 (1.1%) | 7 (2.6%) | +136% |
| fully_passed | 5 (1.1%) | 7 (2.6%) | +136% |

L5 和 fully_passed 有明显提升，说明 TOC/content 修复生效。
L2 持平说明规则解析问题（no_search_results 38%）仍是最大瓶颈。
