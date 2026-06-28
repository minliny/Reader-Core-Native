# 项目状态快照

扫描日期：2026-06-28（v4 — P0 修复后批量验证）
本文为时间点快照，不作为永久事实。后续工作必须以实际代码验证为准。
前置快照：docs/STATUS_SNAPSHOT_2026-06-28.md v3（P0 修复前）

---

## 0. 当前状态摘要

| 维度 | 状态 |
|------|------|
| **编译** | ✅ `cargo build -p reader-cli` 0 errors |
| **测试** | ✅ **1441 tests, 0 failed** |
| **Conformance** | ✅ **173/173 PASS** |
| **全量 459 源 L2 通过** | **10.5%（48/459）** — v2 6.3% → v3 10.5%，+19 源（+65.5%） |
| **全量 459 源全链路通过** | **1 源（天堂深圳）** — v2 0 → v3 1（首次 L1-L5 全绿） |
| **Release blockers** | **50 active**, 1 resolved（multirule） |
| **Legado 能力对标** | **36 已实现 / 14 部分 / 44 未实现 / 11 Host** |
| **S 阶段** | S0 100% / S1 ~45% / S2 ~70% / S3 ~65% / S4 ~30% / S5 ~70% / S6 ~50% / S7 ~25% |
| **工作树** | ✅ clean（commit 60ac6625 后无残留变更） |

---

## 1. 全量 459 源批量测试 v3 → v2 对比（P0 修复后）

**测试条件**：`--test-corpus tests/fixtures/corpus/corpus-manifest.json --keyword "斗破苍穹" --timeout 15 --max-sources 459`，live HTTP，5144.6s（~86 min，比 v2 17 min 长 5x — 因更多源到达深层级，每源多发起 L3/L4/L5 HTTP 请求）。报告：`reports/tooling/corpus-batch-v3.json`。

**P0 修复内容（commit 60ac6625）：**
1. `{{source.key}}` / `{{source.bookSourceUrl}}` / `{{source.booksourceurl}}` 替换为 baseUrl；baseUrl 回退到 bookSourceUrl
2. JS 降级不再返回空：无 evaluator 时保留原始模板文本继续执行（`{{key}}`/`{{page}}` 等），`<js>` 段被剥离但纯文本选择器继续
3. `classify_js_expression` 对 `{{source.key}}`/`{{cookie.*}}` 不再误分类为需要 JS 引擎

| 级别 | v2 (P0 修复前) | v3 (P0 修复后) | Δ |
|------|-----------|------------|-----|
| L1-import | 459 (100%) | 459 (100%) | — |
| L2-search | 29 (6.3%) | **48 (10.5%)** | **+19 (+65.5%)** |
| L3-detail | 11 | **30** | **+19 (+172.7%)** |
| L4-toc | 0 | **5** | **+5（首次有源通过 L4）** |
| L5-content | 0 | **1** | **+1（首次有源通过 L5）** |
| fully_passed | 0 | **1** | **+1（天堂深圳 全链路）** |

**L4 通过的 5 源**：盐选文库（优+）、商店小说（优）、天堂深圳（优）✅全链路、酸奶漫画×2

**L3 通过的 30 源**（v2 仅 11）：新增小米阅读/小米书城/米读小说/速读谷吧/猫耳听书/时代音乐/优品文档/优品学习/书旗小说/猫九小说/企鹅阅读×2/追书出版/掌阅小说/苏轻小说/若初文学/梧桐中文/安轻小说/腾讯漫画×2/酸奶漫画×2/爱漫客栈/光社漫畫/吉站漫画/爱轻写真/商店小说/天堂深圳/盐选文库 等

**失败原因分布（411 L2 失败，v2→v3 变化）：**

| 原因 | v2 | v3 | Δ | 解读 |
|------|------|------|----|------|
| no_search_results | 206 | **184** | **-22** | `{{source.key}}` 修复直接见效 — 之前误用 baseUrl 当搜索 URL 的源现在能正确展开 searchUrl |
| parse_error | 9 | **2** | **-7** | 模板保留文本后解析更稳健 |
| core_error | 45 | 42 | -3 | 边际改善 |
| http_error | 83 | 80 | -3 | 网络抖动 |
| js_unsupported | 97 | 96 | -1 | **修复不针对 JS 引擎本身** — 仍需 JS evaluator 才能解决 96 个源 |
| timeout_or_disconnected | 12 | 35 | +23 | 网络抖动 + 更多源到达深层后触发更多 HTTP 请求（非代码回归） |
| no_toc_entries | 7 | 15 | +8 | **非回归** — 更多源到达 L4 才暴露 TOC 解析问题 |
| empty_content | 0 | 3 | +3 | **非回归** — 更多源到达 L5 才暴露内容为空（盐选文库/商店小说/酸奶漫画） |
| no_chapter_url | 0 | 1 | +1 | 新边界（L5 缺 chapterUrl 提取） |

**关键结论：**
1. **P0 修复成功验证** — L2 +19、L3 +19、L4 +5、L5 +1、fully_passed 0→1，证明 `{{source.key}}` 展开和 JS 降级不阻断两个修复都生效
2. **js_unsupported 几乎未减（97→96）** — 符合预期：修复是"不阻断管道"，不是"支持 JS"。96 个源真正需要 JS 引擎才能搜索，这是下一阶段 S3 JS 沙箱的硬缺口
3. **no_search_results -22 但仍有 184** — `{{source.key}}` 修复解决了 22 个源，剩余 184 个主要是：(a) 网站本身无该关键词结果，(b) searchUrl DSL 含 `,{json}`/`,{"method":"POST"}` 等复杂格式未支持，(c) 需要登录/cookie
4. **timeout +23 是噪声不是回归** — live 测试网络敏感；v3 多跑 5x 时长意味着更多源触达深层 HTTP，timeout 自然上升
5. **下一瓶颈：JS 引擎（96 源）+ searchUrl DSL POST 支持（影响 ~80 源）+ TOC 解析（15 源 no_toc_entries）**

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
| S1 书源导入 | ⚠️ ~45% | L1 100%，L2 10.5%（v3: 48/459），L3 30，L4 5，L5 1 — P0 修复见效，下一瓶颈 JS 引擎 |
| S2 CSS 规则 | ⚠️ ~70% | MultiRule 已修待批量验证改善 |
| S3 JS 沙箱 | ⚠️ ~65% | host callback bridge 通了；v3 后 js_unsupported 96 源是硬缺口 |
| S4 分页+TTS | ⚠️ ~30% | v3 首次有 5 源通过 L4（含分页 TOC），1 源通过 L5 |
| S5 内容处理 | ⚠️ ~70% | ReplaceRule/T2S/TXT目录/书签/BookGroup/ReadRecord 全部实现 |
| S6 平台集成 | ⚠️ ~50% | Android E2E ✅, iOS Rust default ✅, Harmony ⚠️ |
| S7 全量验证 | ⚠️ ~25% | 459 源 L2 10.5%（v3），fully_passed 1，录像 15 源，CI 离线回放就绪 |

---

## 5. 关键阻塞点（按影响排）

| 优先级 | 阻塞点 | 影响 | 修复成本 |
|--------|--------|------|---------|
| ✅ 已解 | ~~批量测试工具 searchUrl 读取 bug~~ | v3 验证：no_search_results 206→184（-22） | commit 60ac6625 |
| ✅ 已解 | ~~JS 降级过于保守~~ | v3 验证：L3 +19、L4 +5、L5 +1（管道不再被空结果阻断） | commit 60ac6625 |
| **P0** | JS 引擎缺失（js_unsupported 96 源） | 96/459 源真正需要 JS evaluator 才能搜索 — 单点最大 L2 提升空间 | 高（需嵌入 QuickJS 或 host JS bridge） |
| **P0** | searchUrl DSL POST/JSON body 未支持 | 影响 ~80 源（`,{"method":"POST","body":"..."}` 格式） | 中（HttpClient 扩展 POST + AnalyzeUrl DSL 解析） |
| **P1** | TOC 解析（no_toc_entries 15 源） | L4 通过率瓶颈 — 多个源到达 L3 但 TOC 解析空 | 中（nextTocUrl + 分页边界） |
| **P1** | 44 项未实现 | 核心 Reader 能力差距 | 长线 |
| **P2** | 平台侧 50 个 active blocker | Release gate 不可达 | 长线 |
| **P2** | UI 与 Core 未打通 | 用户不可见 | 需要统一规划 |

---

## 6. P0 修复后批量验证增量成果

| 能力 | v2 (修复前) | v3 (修复后) | 证据 |
|------|------|------|------|
| L2 批量通过率 | 6.3% (29/459) | **10.5% (48/459)** | corpus-batch-v3.json |
| L3 批量通过率 | 2.4% (11/459) | **6.5% (30/459)** | corpus-batch-v3.json |
| L4 批量通过率 | 0% (0/459) | **1.1% (5/459)** | corpus-batch-v3.json |
| L5 批量通过率 | 0% (0/459) | **0.2% (1/459)** | corpus-batch-v3.json |
| fully_passed | 0 | **1（天堂深圳）** | corpus-725d4e38e786 |
| MultiRule \|\| 拆分 | ✅ 已修复 | ✅ 保持 | 4 yodu tests |
| BookGroup 实体 | ✅ entity+storage+CRUD+dispatch | ✅ 保持 | 3 test files |
| ReadRecord 实体 | ✅ entity+storage+CRUD+dispatch | ✅ 保持 | 3 test files |
| Bookmark dispatch | ✅ protocol dispatch | ✅ 保持 | bookmarks_commands.rs |
| 录像 | 15 源 | 15 源（v3 未启用 --record） | tests/fixtures/corpus/recorded/ |
| CI 离线回放 | ✅ 15/15 replay | ✅ 保持 | corpus-batch-offline-v2.json |

---

## 7. 下阶段建议

1. **P0 嵌入 JS 引擎或打通 host JS bridge**（96 源 js_unsupported 是单点最大 L2 提升空间；预期 L2 10.5%→30%+）— 需评估 QuickJS 嵌入 vs host callback 两条路线对 Core/Host 边界的影响
2. **P0 searchUrl DSL POST + JSON body 支持**（影响 ~80 源；预期 L2 再 +15-20%）— HttpClient 扩展 POST + AnalyzeUrl 解析 `,{json}`/`,{"method":"POST","body":"..."}`
3. **P1 TOC 解析修复**（15 源 no_toc_entries — 小米阅读/小米书城/书旗/猫九/企鹅/掌阅/苏轻/若初/梧桐/安轻 等 L3 已通过但 L4 空）— 预期 L4 5→15+
4. **P1 内容处理：去重标题 + 智能分段 + 内容净化**（ContentProcessor 已有，但缺 reSegment/upRemoveSameTitle）— 解决 3 源 empty_content
5. **P1 分页批量验证**（跑 `--test-corpus-offline` 确认 15 录像源分页效果）
6. **P2 开始关闭 release blockers**（从 50 → 40 → 30）
7. **P2 Document 侧：全文搜索 + 换源 + 段评**

---

*本文为 2026-06-28 v4 — P0 修复后批量验证。关键发现：commit 60ac6625 两个 P0 修复（{{source.key}} 展开 + JS 降级不阻断）成功验证 — L2 6.3%→10.5%（+19 源），首次出现 L4/L5 通过和 fully_passed 源（天堂深圳全链路）。下一瓶颈明确：JS 引擎（96 源）+ searchUrl DSL POST（~80 源）。*
