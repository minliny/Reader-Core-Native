# 项目状态快照

扫描日期：2026-06-28（v3）
本文为时间点快照，不作为永久事实。后续工作必须以实际代码验证为准。
前置快照：docs/STATUS_SNAPSHOT_2026-06-28.md v2

---

## 0. 当前状态摘要

| 维度 | 状态 |
|------|------|
| **编译** | ✅ `cargo check --workspace` 0 errors, `cargo build -p reader-ffi --release` 成功 |
| **测试** | ✅ **1435 tests, 0 failed**（10 crate + doc-tests）|
| **Conformance** | ✅ **173/173 PASS** |
| **全量 459 源 L2 通过** | **6.3%**（v1: 2.2% → +19 源，3x改善） |
| **Release blockers** | **50 active**, 1 resolved（multirule） |
| **Legado 能力对标** | **36 已实现 / 14 部分 / 44 未实现 / 11 Host** |
| **S 阶段** | S0 100% / S1 ~40% / S2 ~70% / S3 ~65% / S4 ~25% / S5 ~70% / S6 ~50% / S7 ~15% |
| **工作树** | ✅ clean（5 批 commit 后无残留变更） |

---

## 1. 全量 459 源批量测试 v2 → v1 对比

| 级别 | v1 (修复前) | v2 (修复后) | Δ |
|------|-----------|------------|-----|
| L1-import | 459 (100%) | 459 (100%) | — |
| L2-search | 10 (2.2%) | 29 (6.3%) | **+19 (3x)** |
| L3-detail | 2 | 11 | **+9 (5x)** |
| L4-toc | 0 | 0 | — |
| L5-content | 0 | 0 | — |

**失败原因分布（430 L2 失败）：**

| 原因 | 数量 | 占比 | 根因分析 |
|------|------|------|----------|
| no_search_results | 206 | 48% | **搜索 URL 构造/模板未展开** — 源有真实 searchUrl（含 {{key}} / ,{json} DSL），但批量测试工具未正确读取 searchUrl 字段，用了 baseUrl 当搜索 URL |
| js_unsupported | 97 | 23% | **{{}}/<js>/@js: 模板降级为空** — execute_legado_rule 中 `substitute_js_templates` 在无 JS evaluator 时返回空（graceful degradation 导致结果为空而非报错） |
| http_error | 83 | 19% | 真实网络问题 |
| core_error | 45 | 10% | Core 解析/执行错误（含 CSS parse error、MultiRule 残余等） |
| timeout/disconnected | 12 | 3% | 网络超时 |
| parse_error | 9 | 2% | 响应解析错误 |
| no_toc_entries | 7 | 2% | TOC 解析空（含分页未覆盖边界） |

**关键根因结论：**
1. **批量测试工具自身的 URL 构造 bug** — 没有读取 `searchUrl` 字段，用了 `source_url`（baseUrl）作为 HTTP 请求。修复这个 bug 预计能提升 L2 通过率到 50%+
2. **JS 降级逻辑过保守** — `{{}}`/`<js>` 无 evaluator 时返回空而不是发 HTTP 裸请求。需调整为：模板/JS 无法展开时，直接发 base URL 的 GET 请求
3. **no_search_results 206 个源中 161 个是 "plain_http"** — 其实它们有真实的 searchUrl（`/modules/article/search.php,{"body":"searchkey={{key}}...","method":"POST"}` 等 DSL 格式）

---

## 2. 五方状态矩阵

| 维度 | Core (Rust) | Android | iOS | HarmonyOS | UI |
|------|------------|---------|-----|-----------|-----|
| 构建 | ✅ 1435 tests 0 failed | ✅ `.a` CMake | ✅ shell smoke 110/110 | ⚠️ 页面级 | frontend-demo |
| Rust Core 集成 | — | ✅ JNI + EndToEndTest | ✅ ServiceMode.rustCore 默认 | ✅ ReaderCoreClient facade | ❌ 未打通 |
| E2E 测试 | ❌ 6.3%/459源 | ✅ 8/8 emulator | ✅ 110/110 | ❌ 无自动化 | ❌ |
| 核心缺口 | 44项未实现 | TTS/文件/登录 | 后台/签名/登录 | 主题/后台/登录 | 与 Core 无连接 |
| 最新 commit | `542fddc5` | `19dfdab` | `a845066` | `338205f` | `1a635ec` |

---

## 3. Legado 能力对标（97 项）

| 分类 | 总项 | 已实现 | 部分 | 未实现 | Host |
|------|------|--------|------|--------|------|
| 规则引擎 | 18 | 9 | 4 | 5 | 0 |
| URL 构造 | 8 | 3 | 3 | 2 | 0 |
| 书源生命周期 | 7 | 4 | 1 | 2 | 0 |
| JS 扩展方法 | 19 | 5 | 3 | 9 | 2 |
| 本地书 | 6 | 3 | 1 | 2 | 0 |
| RSS | 6 | 2 | 2 | 2 | 0 |
| TTS | 4 | 1 | 0 | 2 | 1 |
| 同步与备份 | 4 | 1 | 2 | 1 | 0 |
| 数据实体 | 10 | 4 | 0 | 6 | 0 |
| 内容处理 | 5 | 0 | 0 | 5 | 0 |
| 阅读引擎 | 5 | 0 | 0 | 0 | 5 |
| 图片/封面 | 5 | 0 | 0 | 3 | 2 |
| **合计** | **97** | **36** | **14** | **37** | **10** |

**Core 侧完成度：~44%**（36+14=50/97，含部分实现）

---

## 4. S 阶段进度

| 阶段 | 进度 | 依据 |
|------|------|------|
| S0 协议 | ✅ 100% | 173 conformance |
| S1 书源导入 | ⚠️ ~40% | L1 100% 但 L2 仅 6.3%（测试工具 bug + JS 降级过保守） |
| S2 CSS 规则 | ⚠️ ~70% | MultiRule 已修待批量验证改善 |
| S3 JS 沙箱 | ⚠️ ~65% | host callback bridge 通了，但 JS 降级策略需调 |
| S4 分页+TTS | ⚠️ ~25% | 分页已修但无批量验证 |
| S5 内容处理 | ⚠️ ~70% | ReplaceRule/T2S/TXT目录/书签/BookGroup/ReadRecord 全部实现 |
| S6 平台集成 | ⚠️ ~50% | Android E2E ✅, iOS Rust default ✅, Harmony ⚠️ |
| S7 全量验证 | ⚠️ ~15% | 459 源 6.3%，录像 15 源，CI 离线回放就绪 |

---

## 5. 关键阻塞点（按影响排）

| 优先级 | 阻塞点 | 影响 | 修复成本 |
|--------|--------|------|---------|
| **P0** | 批量测试工具 searchUrl 读取 bug | 206 源误报 no_search_results，预期修复后 L2 可达 50%+ | 低（reader-cli 中读取 sc.searchUrl 而不是 doc.baseUrl） |
| **P0** | JS 降级过于保守 | 97 源因 {{}}/{{key}} 模板无法展开返回空结果 | 低（无 JS 时发裸 GET 而非返回空） |
| **P1** | HttpClient 不支持 POST + JSON body DSL（`,{"method":"POST","body":"..."}`） | 部分源搜索需要 POST | 中 |
| **P1** | 44 项未实现 | 核心 Reader 能力差距 | 长线 |
| **P2** | 平台侧 50 个 active blocker | Release gate 不可达 | 长线 |
| **P2** | UI 与 Core 未打通 | 用户不可见 | 需要统一规划 |

---

## 6. 5 个 agent 执行后增量成果

| 能力 | 之前 | 现在 | 证据 |
|------|------|------|------|
| MultiRule || 拆分 | ❌ 丢失 | ✅ 已修复 | 4 yodu tests, 1435/0 |
| BookGroup 实体 | ❌ 0 代码 | ✅ entity+storage+CRUD+dispatch | 3 test files |
| ReadRecord 实体 | ❌ 0 代码 | ✅ entity+storage+CRUD+dispatch | 3 test files |
| Bookmark dispatch | ❌ 空 handler | ✅ protocol dispatch | bookmarks_commands.rs |
| L2 批量通过率 | 2.2% (10/459) | 6.3% (29/459) | corpus-batch-v2.json |
| L3 批量通过率 | 0.4% (2/459) | 2.4% (11/459) | corpus-batch-v2.json |
| 录像 | 1 源 | 15 源 | tests/fixtures/corpus/recorded/ |
| CI 离线回放 | ❌ 不可用 | ✅ 15/15 replay | corpus-batch-offline-v2.json |

---

## 7. 下阶段建议

1. **P0 修复批量测试工具 searchUrl bug**（1 天，预期 L2 25%→50%+）
2. **P0 调整 JS 降级逻辑**（1 天，预期 L2 再 +10-20%）
3. **P1 内容处理：去重标题 + 智能分段 + 内容净化**（ContentProcessor 已有，但缺 reSegment/upRemoveSameTitle）
4. **P1 分页批量验证**（跑 `--test-corpus-offline` 确认 15 录像源分页效果）
5. **P2 开始关闭 release blockers**（从 50 → 40 → 30）
6. **P2 Document 侧：全文搜索 + 换源 + 段评**

---

*本文为 2026-06-28 v3 全量审计。关键发现：批量测试工具自身 bug 是 L2 最大失败原因，修复后 459 源通过率预期可大幅提升。*
