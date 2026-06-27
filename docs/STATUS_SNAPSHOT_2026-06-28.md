# 项目状态快照

扫描日期：2026-06-28
本文为时间点快照，不作为永久事实。后续工作必须以实际代码验证为准。
前置快照：docs/STATUS_SNAPSHOT_2026-06-27.md
审计版本：2（2026-06-28 跨层全量审计，含五方状态 + 12 未提交变更）

---

## 1. 已完成 Agent 结果汇总

本轮 7 个能力缺口 agent 全部完成（遗留书源测试工具链 agent + BookGroup/ReadRecord 未完成）：

### Agent 1 — MultiRule 操作符拆分 ✅ RESOLVED

| 项 | 值 |
|----|-----|
| Blocker | rb-legado-css-multirule-operator |
| 状态 | ✅ 修复，标记 evidence_status: fixed |
| 测试 | 15 multirule tests + 4 yodu fixture tests pass |
| 代码 | reader-rule/src/lib.rs (+442), reader-content/src/lib.rs (+35) |
| 修复内容 | split_legado_combined_rule 在分发层统一分割 &&/||/%% (CSS/XPath/Regex/JSONPath)；裸抽取词识别(text/href等)；list 模式返回 outer HTML；fragment 根元素 |
| Conformance | 173/173 pass |
| 遗留 | release-blockers.json 中 blocker 计数仍显示 1（JSON 未更新计数，但条目已标记 RESOLVED） |

### Agent 2 — 规则补全 RuleComplete ✅

| 项 | 值 |
|----|-----|
| 状态 | ✅ 完成 |
| 测试 | 32 tests pass (legado_rule_complete.rs) |
| 代码 | reader-rule/src/lib.rs: auto_complete_rule()；reader-content/src/lib.rs: extract_rule_value 集成 |
| 修复内容 | 1:1 对齐 Legado RuleComplete.kt autoComplete()：needComplete 正则识别缺尾操作符；notComplete 跳过复杂规则；fixImgInfo img@text→img@alt；XPath 用 //text()//@href//@src |
| 批量测试影响 | 预期减少 no_search_results 失败（449 个 L2 失败中 206 个是 no_search_results） |

### Agent 3 — 多页加载 nextTocUrl/nextContentUrl ✅

| 项 | 值 |
|----|-----|
| 状态 | ✅ 完成 |
| 测试 | 8 tests pass (pagination.rs) |
| 代码 | reader-runtime/src/remote.rs: book_toc_from_params / chapter_content_from_params 循环翻页 |
| 修复内容 | 检查 nextTocUrl/nextContentUrl → 发 http.execute → 解析更多 → 合并/拼接 → 循环到无下一页；环检测(HashSet)；MAX_NEXT_PAGES=50 防死循环 |
| 批量测试影响 | L4-toc/L5-content 可达（之前全 skip） |

### Agent 4 — URL JS host callback bridge ✅

| 项 | 值 |
|----|-----|
| 状态 | ✅ 完成 |
| 测试 | 140 tests pass (含 8 新 bridge tests) |
| 代码 | reader-runtime/src/host_callback_bridge.rs (643行新建)；remote.rs: RemoteState 持有 bridge；runtime.rs: send() 拦截 host.complete |
| 修复内容 | 方案B(同步阻塞+send-time拦截)：JS 中 java.get/post/ajax → HostCallbackBridge → host.request → worker 阻塞等待 Condvar → send() 拦截 host.complete 唤醒 → 返回 body 给 JS |
| 批量测试影响 | 预期减少 url_js_failed（449 个 L2 失败中 90 个是 URL JS 失败） |

### Agent 5 — 替换规则 ReplaceRule ✅

| 项 | 值 |
|----|-----|
| 状态 | ✅ 完成 |
| 测试 | 15 content_processor tests + 9 replace_rule_commands tests pass |
| 代码 | reader-content/src/content_processor.rs (170行新建)；reader-contract: 4 method 常量 + DTO；reader-runtime: 4 dispatch + 6 handler；protocol: 4 replace-rule.* + $defs；reader-storage: replace_rules 表 |
| 修复内容 | ContentProcessor 对照 Legado ContentProcessor.kt:91 getContent()：繁简转换 → trim → scope 匹配 → order 排序 → regex/字符串替换 |

### Agent 6 — 繁简转换 t2s/s2t ✅

| 项 | 值 |
|----|-----|
| 状态 | ✅ 完成 |
| 测试 | 98 reader-js tests + 53 reader-content tests pass |
| 代码 | reader-content/src/chinese.rs (114行新建)；reader-js/src/lib.rs: t2s/s2t 用 zhhz crate；Cargo.toml: zhhz = "0.4" |
| 修复内容 | zhhz(纯 Rust OpenCC 重实现) 替换 stub；java.t2s("測試")=="测试"；ContentProcessor 按 chineseConverterType 在替换前执行转换 |

### Agent 7 — 发现 Explore + TXT 目录规则 TxtTocRule ✅

| 项 | 值 |
|----|-----|
| 状态 | ✅ 完成 |
| 测试 | 10 explore_kinds tests + 6 split_chapters tests pass |
| 代码 | reader-content/src/lib.rs: parse_explore_kinds() + explore()；reader-local-book/src/txt.rs: split_chapters()；reader-contract: SOURCE_EXPLORE_KINDS/SOURCE_EXPLORE + TXT_TOC_RULE_* 常量+DTO；protocol: source.explore + txt-toc-rule.* + $defs；reader-storage: txt_toc_rules 表 |
| 修复内容 | exploreUrl 解析(JSON数组 + 名称::url + @js:)；TXT 按 TxtTocRule 正则分章(regex find + 1000-char gap 去重 + fallback 单章) |

### Agent 8 — 书签 + 数据实体 ⚠️ 部分

| 项 | 值 |
|----|-----|
| 状态 | ⚠️ Bookmark 完成(Bookmark 实体 + bookmarks 表 + CRUD)；BookGroup/ReadRecord 未完成（Agent 因文件冲突中断，0 代码） |
| 代码 | reader-domain: Bookmark struct(842行)；reader-storage: bookmarks 表(190行) + put_bookmark/row_to_bookmark |
| 未完成 | BookGroup 实体/表/协议；ReadRecord 实体/表/协议；bookmark.*/book-group.*/read-record.* protocol 方法 |

---

## 2. 全量 459 源批量测试结果（真实数据）

### corpus-batch-full.json（459 源 live 网络）

| 级别 | passed | failed | skipped |
|------|--------|--------|---------|
| L1-import | 459 (100%) | 0 | 0 |
| L2-search | 10 (2.2%) | 449 | 0 |
| L3-detail | 2 | 8 | 449 |
| L4-toc | 0 | 2 | 457 |
| L5-content | 0 | 0 | 459 |

**完全通过: 0/459 = 0%**
**部分通过: 459/459 = 100%**（全部 L1 通过）

### L2-search 449 个失败原因分布

| 类别 | 数量 | 占比 | 对应缺口 |
|------|------|------|---------|
| no_search_results | 206 | 46% | 规则补全(Agent 2 已修，未重跑验证) |
| url_js_failed | 90 | 20% | URL JS host callback(Agent 4 已修，未重跑验证) |
| network_error | 47 | 10% | 真实网络超时/连接失败(非 Core 问题) |
| css_parse_error | 37 | 8% | CSS 选择器解析(MultiRule 已修，可能还有边界) |
| other | 65 | 15% | 空错误/模板变量未展开/cookie 引用等 |
| url_parse_error | 4 | 1% | URL DSL 解析 |

### 关键发现

1. **L1-import 100%** — 459 源全部能被 Core 导入
2. **L2-search 2.2%** — 真实书源搜索几乎全失败
3. **最大原因 no_search_results 46%** — Agent 2(规则补全)已修，但**未重跑批量测试验证**
4. **第二原因 url_js_failed 20%** — Agent 4(host callback)已修，但**未重跑批量测试验证**
5. **network_error 10%** — 真实网络问题，非 Core 缺口
6. **7 个 agent 的修复全部未重跑批量测试验证** — 无法确认真实改善

---

## 3. 能力清单更新（97 项）

### 本轮新增已实现能力（10 项）

| # | 能力 | Legado 对标 | 证据 |
|---|------|------------|------|
| 1 | MultiRule &&/||/%% 拆分 | AnalyzeRule.splitSourceRule | 15+4 tests + yodu 真实源 |
| 2 | 规则补全 RuleComplete | RuleComplete.autoComplete | 32 tests |
| 3 | 多页加载 nextTocUrl | BookChapterList:192 | 8 tests |
| 4 | 多页加载 nextContentUrl | BookContent:185 | 8 tests |
| 5 | URL JS host callback | AnalyzeUrl:153 + JsExtensions | 140 tests(含端到端) |
| 6 | 替换规则 ReplaceRule | ReplaceRule + ContentProcessor:91 | 15+9 tests |
| 7 | 繁简转换 t2s/s2t | ChineseUtils + JsExtensions:547 | 98+53 tests |
| 8 | 发现 Explore | WebBook:93 + BookSourceExtensions:44 | 10 tests |
| 9 | TXT 目录规则 TxtTocRule | TxtTocRule + TextFile:440 | 6 tests |
| 10 | 书签 Bookmark | Bookmark.kt | 实体+表+CRUD |

### 更新后能力统计

| 状态 | 之前 | 现在 | 变化 |
|------|------|------|------|
| 已实现 | 22 | 32 | +10 |
| 部分实现 | 16 | 16 | — |
| 未实现 | 45 | 35 | -10 |
| Host/UI 层 | 14 | 14 | — |

---

## 4. S 阶段进度（基于全量批量测试修正）

| 阶段 | 之前快照 | 修正后 | 依据 |
|------|---------|--------|------|
| S0 | 100% | ✅ 100% | — |
| S1 | 35% | ⚠️ ~45% | L1 100%，L2 2.2%(7个修复未重跑验证，预期提升) |
| S2 | 55% | ⚠️ ~70% | MultiRule+RuleComplete 已修，待批量验证 |
| S3 | 40% | ⚠️ ~65% | host callback bridge 已通，待批量验证 |
| S4 | 15% | ⚠️ ~25% | 多页加载已修，但 L4/L5 批量仍全 skip(未重跑) |
| S5 | 50% | ⚠️ ~65% | 替换规则+繁简+TXT目录+书签已实现 |
| S6 | 50% | ⚠️ ~50% | 平台侧无变化 |
| S7 | 5% | ⚠️ ~10% | 全量 459 源已跑(0% 通过)，但修复后未重跑 |

**核心问题：7 个 agent 的修复全部未重跑批量测试验证，无法确认真实改善。**

---

## 5. 仓库未提交变更状态

12 个文件变更（9 modified + 4 new），全部未提交。按 agent 分组：

### MultiRule Agent
- M: crates/reader-rule/src/lib.rs, crates/reader-content/src/lib.rs, reader-rule/tests/*

### RuleComplete Agent
- M: reader-rule/src/lib.rs (叠加), reader-content/src/lib.rs (叠加)

### Pagination Agent
- M: reader-runtime/src/remote.rs, reader-runtime/src/runtime.rs
- ?? reader-runtime/tests/pagination.rs

### Host Callback Agent
- M: reader-runtime/src/remote.rs (叠加), runtime.rs (叠加), reader-runtime/Cargo.toml, reader-runtime/src/lib.rs, reader-js/src/lib.rs, reader-js/Cargo.toml
- ?? reader-runtime/src/host_callback_bridge.rs

### ReplaceRule Agent
- M: reader-content/src/lib.rs (叠加), reader-content/Cargo.toml, reader-contract/src/lib.rs, reader-contract/src/remote.rs, reader-runtime/src/remote.rs (叠加), protocol/reader-command.schema.json
- ?? reader-content/src/content_processor.rs, reader-content/tests/content_processor.rs, reader-runtime/tests/replace_rule_commands.rs

### Chinese t2s/s2t Agent
- M: Cargo.toml, Cargo.lock, reader-js/Cargo.toml, reader-js/src/lib.rs (叠加), reader-js/tests/js_runtime_compat.rs, reader-content/Cargo.toml (叠加), reader-content/src/lib.rs (叠加)
- ?? reader-content/src/chinese.rs

### Explore + TxtTocRule Agent
- M: reader-content/src/lib.rs (叠加), reader-contract/src/lib.rs (叠加), reader-contract/src/remote.rs (叠加), reader-runtime/src/remote.rs (叠加), reader-local-book/src/txt.rs, protocol/reader-command.schema.json (叠加), reader-domain/src/lib.rs
- ?? reader-content/tests/explore_kinds.rs

### Bookmark Agent (部分)
- M: reader-domain/src/lib.rs (叠加), reader-storage/src/sqlite_backend.rs

### 测试工具链 Agent (已提交)
- M: reader-cli/Cargo.toml, reader-cli/src/main.rs, .github/workflows/core.yml, reader-content/Cargo.toml (叠加)
- ?? test_source.rs, test_corpus.rs, corpus-batch-*.json, assessment.md, recorded/

### 审计 Agent
- M: release-blockers.json, docs/LEGADO_CAPABILITY_INVENTORY.md

---

## 6. 五方全量审计（本次新增）

### 6.1 审计覆盖范围

| 维度 | 审计对象 |
|------|---------|
| Core | `Reader-Core-Native`（Rust，本项目，main@c3bbdbc4） |
| 旧 Core | `Reader-Core`（Swift，525 文件，已标记迁移） |
| Android | `Reader for Android`（19dfdab，端到端 8/8 PASS） |
| iOS | `Reader for iOS`（a845066，110/110 shell smoke） |
| HarmonyOS | `Reader for HarmonyOS`（338205f，5 UI 页面已切 Rust Core） |
| UI | `Reader UI`（独立仓库，1a635ec，frontend-demo 未打通 Core） |
| Mac/Win | 仅有占位目录，无实质代码 |

### 6.2 平台侧状态矩阵

| 能力 | Android | iOS | HarmonyOS |
|------|---------|-----|-----------|
| Rust Core 链接 | ✅ `.a`+JNI CMake | ✅ ServiceMode.rustCore 默认 | ✅ ReaderCoreClient facade |
| 端到端测试 | ✅ 8/8 PASS (emulator) | ✅ 110/110 shell smoke | ⚠️ 页面级可用，无自动化测试 |
| HTTP 传输 | ✅ OkHttpHostTransport | ✅ URLSession+HostRequestRouter | ⚠️ HTTPAdapter 有代码但 capability 分发未通 |
| Cookie 管理 | ✅ AndroidCookieManagerStore | ✅ WKWebView cookie mirror | ✅ ArkWeb WebCookieManager |
| 安全凭据 | ✅ AndroidKeyStore | ✅ Keychain | ✅ HUKS |
| TTS | ❌ 仅 Fake | ✅ AVSpeechSynthesizer | ✅ SystemTtsAdapter |
| 文件选择 | ⚠️ ContentResolver 无 SAF 入口 | ✅ fileImporter | ✅ FilePickerAdapter |
| 主题/字体 | ✅ ReaderTheme | ✅ ReaderTheme full | ❌ 硬编码 |
| 登录 WebView | ❌ | ⚠️ supportsLoginFlow:false | ⚠️ 有 WebView 但无登录流 |
| 包体签名分发 | ❌ | ❌ | ❌ |
| 后台任务/通知 | ⚠️ 仅有 Notification | ❌ | ❌ |
| 当前分支 | codex/android-real-core-runtime-evidence | codex/ios-real-app-core-evidence | main |

### 6.3 旧 Core（Swift）迁移进度

| 维度 | 数据 |
|------|------|
| Swift 源码 | 525 个 `.swift`，41 个 Sources 目录 |
| 核心模块 | ReaderCoreFoundation/Parser/Network/Models/Services/Cache/JSRenderer/API/Bridge/PlatformAdapters/Protocols |
| Rust 替代 | 11 个 crate，~1446 tests |
| 迁移标志 | `a6db53e0 docs: add Reader-Core to Rust migration ledger` |
| **迁移率估算** | **~50%** |
| 已迁移 | 规则引擎(analyzeRule equivalent)、网络协议层、JS 沙箱、协议层(reader-contract)、数据模型(reader-domain)、持久化(reader-storage)、同步(reader-sync) |
| 未迁移 | 登录流支持、WebView 交互、字体反混淆(QueryTTF)、封面解密(coverDecodeJs)、WebSocket 调试、HTTP 调试接口、GlideUrl、漫画阅读、听书、SearchContent、多格式导入导出 |
| 已超过 Swift Core | 替换规则(ContentProcessor)、繁简转换(t2s/s2t)、TXT 目录规则(TxtTocRule)、多页加载(pagination)、测试工具链(reader-cli)、完整 CI 流程 |

### 6.4 Core 侧遗留缺口汇总

**P0（尚未闭环的阻断级缺口）：**
- **全量批量测试验证** — 7 个修复未重跑，当前 0% 完全通过
- **12 个变更未提交** — 已修能力不可落地
- **release-blockers.json blocker 计数错误** — 显示 1 但实际 0

**P1（核心能力缺口）：**
- BookGroup/ReadRecord 数据实体（Agent 8 中断）
- ContentProcessor 内容净化(ContentHelp.kt reSegment)
- 去重标题(upRemoveSameTitle)
- 内容清洗(CSS/HTML 标签剥离)

**P2（进阶能力）：**
- 段评 ReviewRule
- 字体反混淆 QueryTTF
- 封面解密 coverDecodeJs
- 全文搜索 SearchContent
- 换源 ChangeSource
- DictRule/HttpTTS/RuleSub 数据实体

**Host 层缺口（三平台共缺）：**
- 包体签名分发（三平台均无）
- 后台任务/通知（三平台缺或不全）
- 登录 WebView（三平台缺或不全）

### 6.5 测试工具链状态

| 工具 | 状态 | 备注 |
|------|------|------|
| CLI `--test-source` | ✅ 已实现 | 接受 Legado JSON 源 + keyword → L1-L5 链式 |
| CLI `--test-corpus` | ✅ 已实现 | 批量 L1-L5 live 测试 + 自动录像 |
| CLI `--test-corpus-offline` | ✅ 已实现 | 批量 L1-L5 离线回放 |
| corpus-manifest.json | ✅ 459 源全量索引 |
| 459 源脱敏导入 | ✅ 全部 L1 通过 | `tests/fixtures/corpus/sources/` 已就绪 |
| 录像 | ⚠️ 仅 1 源有录像 | offline 回放不可用，需 live 重跑 |
| Python 工具脚本 | ❌ 17 个全部未跑过 | `tools/` 下工具无产出 |
| 批量测试结果 | ✅ corpus-batch-full.json | 截至 agent 修复前的实测值（0%） |
| CI workflow | ⚠️ 已扩展但未验证 | `.github/workflows/core.yml` 含 test-corpus |
| assessment.md | ✅ 已生成 | `reports/tooling/assessment.md` |

---

## 7. 关键风险

| 风险 | 概率 | 影响 | 缓解 |
|------|------|------|------|
| reader-content/src/lib.rs 提交冲突 | 高 | 7 个修复无法合并 | 需手动合并 5 agent 叠加代码 |
| 7 个修复未提效 | 中 | 重跑 459 源仍 <10% | 逐失败原因 debug |
| BookGroup/ReadRecord 永远搁置 | 中 | 数据实体缺口 | 下一轮优先补 |
| 平台侧 blocker 51 条 | 高 | release gate 不可达 | 逐项关闭 |
| UI 与 Core 未打通 | 中 | 用户不可见 | 下一轮 |

---

## 8. 下一步建议

1. **P0** 提交 12 个未提交变更（分批 commit，先处理无冲突的）
2. **P0** 重跑全量 459 源批量测试（live 或离线）验证 7 个修复效果
3. **P1** 补 BookGroup/ReadRecord 数据实体
4. **P1** 确认 reader-content/src/lib.rs 多 agent 叠加无逻辑冲突
5. **P2** 更新 release-blockers.json multirule 计数
6. **P2** 生成 10+ 源的录像用于 CI 离线回放
7. **持续** 对照 35 项未实现 + 16 项部分实现逐项补齐

---

*本文为 2026-06-28 v2 跨层全量审计。7 个 agent 修复全部未重跑批量测试验证，
通过率改善为预期值非实测值。*
