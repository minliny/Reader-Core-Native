# 项目状态快照

扫描日期：2026-06-27（第三次更新，含 MultiRule 修复 + 测试工具链首批结果）
本文为时间点快照，不作为永久事实。后续工作必须以实际代码验证为准。

---

## 1. 仓库状态

| 仓库 | 分支 | 最新 commit | 状态 |
|------|------|-------------|------|
| Core (Reader-Core-Native) | main | 35d12372 | 有多个 agent 未提交变更（见下） |
| Android (Reader for Android) | codex/android-real-core-runtime-evidence | 19dfdab | 已 push，8/8 instrumented test pass |
| iOS (Reader for iOS) | codex/ios-real-app-core-evidence | a845066 | 已 push，110/110 smoke test pass |
| HarmonyOS (Reader for HarmonyOS) | codex/harmony-signed-device-runtime | 338205f | host/ 11文件 + api/ 4文件，BUILD SUCCESSFUL |

### Core 仓库未提交变更（多 agent 并发）

| 变更 | Agent | 文件 | 状态 |
|------|-------|------|------|
| MultiRule 修复 | MultiRule agent | reader-rule/src/lib.rs (+442), reader-content/src/lib.rs (+35), legado_multirule_operator.rs (273行), yodu_multirule_fixture.rs (162行) | ✅ 完成，测试通过，未提交 |
| 测试工具链 | 测试工具 agent | reader-cli/Cargo.toml (+7), main.rs (+274), test_source.rs (1020行), test_corpus.rs (388行), core.yml (+231) | ⚠️ WIP，有编译问题 |
| release-blockers | 审计 agent | release-blockers.json (multirule 标记 RESOLVED) | ✅ 完成，未提交 |
| corpus 批量结果 | 测试工具 agent | corpus-batch-p0.json, corpus-batch-offline-test.json, recorded/ (1文件) | ✅ 首批结果已产出 |

---

## 2. MultiRule 修复结果（rb-legado-css-multirule-operator — RESOLVED）

### 修复内容
- `split_legado_combined_rule` 在 `execute_mode` / `execute_legado_rule_list` 分发层统一分割 `&&`/`||`/`%%`
- `||` = OR-fallback（first non-empty branch wins）
- `&&` = AND-merge（concat all branch results）
- `%%` = parallel zip（interleave by index）
- 多类 CSS 简写 `class.X Y Z` → `.X.Y.Z`
- 裸抽取词识别（text/textNodes/ownText/html/all + 属性名）
- list 模式返回 outer HTML（保留外层元素属性）
- fragment 根元素（跳过 html/head/body 包裹）

### 验证
- reader-rule: 130 unit tests + 15 multirule tests = 145 pass
- reader-content: 4 yodu fixture tests pass（search 15 books / detail title 非空 / toc 非空 / content 含"萧炎"）
- conformance: 173/173 pass
- release-blockers.json: blocker 计数 1→0，multirule 标记 `evidence_status: fixed`，severity 降为 low

---

## 3. 测试工具链首批结果（真实数据！）

### corpus-batch-p0.json（30 个 P0 源，live 网络测试）

| 级别 | passed | failed | skipped |
|------|--------|--------|---------|
| L1-import | 30 | 0 | 0 |
| L2-search | 1 | 29 | 0 |
| L3-detail | 0 | 1 | 29 |
| L4-toc | 0 | 0 | 30 |
| L5-content | 0 | 0 | 30 |

**完全通过: 0 / 30 = 0%**
**部分通过: 30 / 30 = 100%**（全部 L1 通过，L2 大面积失败）

### L2-search 失败原因分布（29 个失败）

| 原因 | 数量 | 说明 |
|------|------|------|
| no_search_results | 10 | HTTP 请求成功但解析返回空（规则引擎问题） |
| URL JS 执行失败 | 8 | @js: searchUrl 的 JS 无法执行（host callback 缺失/语法错误/变量未定义） |
| URL DSL parse error | 2 | Legado URL DSL 格式解析失败 |
| invalid JSON input | 2 | 响应体非合法 JSON（JSONPath 规则期望 JSON） |
| invalid CSS selector | 2 | 规则含 `<js>` 被当 CSS selector |
| invalid JSONPath | 1 | MultiRule 在 JSONPath 中未正确处理 |
| Network Error | 2 | 真实网络超时/连接失败 |
| recv timeout | 1 | 12s 超时 |
| Bad URL | 1 | IDN 域名解析失败 |

### 关键发现

1. **L1-import 100% 通过** — 459 源全部能被 Core 导入解析（source.import 无问题）
2. **L2-search 仅 3.3% 通过** — 真实书源搜索大面积失败
3. **最大失败原因: no_search_results (34%)** — HTTP 成功但规则解析返回空
   - 可能原因: 规则补全未实现(RuleComplete)、站点 HTML 变化、编码问题
4. **第二大原因: URL JS 执行失败 (28%)** — @js: searchUrl 的 JS 无法执行
   - 根因: JS sandbox 中 java.get/post 等 host callback 未接通
   - 这不是 MultiRule 问题，是 S3(JS/host) 的真实缺口
5. **MultiRule 修复已生效** — yodu 真实源直驱测试全链路通过，但批量测试中
   仍有 1 个 invalid JSONPath MultiRule 失败（可能是 JSONPath 层的 && 未修）

---

## 4. S 阶段进度（基于真实数据修正）

| 阶段 | 之前声称 | 修正后实际 | 依据 |
|------|---------|-----------|------|
| S0 (baseline) | 100% | ✅ 100% | — |
| S1 (BookSource compat) | 100% | ⚠️ ~35% | L1 100% 但 L2 仅 3.3%，真实源大面积搜索失败 |
| S2 (DSL) | 70% | ⚠️ ~55% | MultiRule 已修(15+4 tests)，但 URL JS 执行未通 |
| S3 (request/JS/host) | 100% | ⚠️ ~40% | 28% 源因 URL JS 失败，java.* host callback 未接通 |
| S4 (remote reading) | 60% | ⚠️ ~15% | 30 源 0% 完全通过，L3-L5 全 skip |
| S5 (data/local/RSS/sync) | 90% | ⚠️ ~50% | 替换规则/繁简/TXT目录等未实现 |
| S6 (strangler migration) | 45% | ⚠️ ~50% | Android simulator proof；iOS smoke 110/110；HarmonyOS build proof |
| S7 (corpus benchmark) | 0% | ⚠️ ~5% | 首次批量测试已跑（30源），但通过率 0% |

---

## 5. Release Blockers 状态

- **blocker: 0**（MultiRule 已 RESOLVED）
- **medium: 6** — 签名/后台任务/登录(all) + 主题(harmony) + SAF(android) + TTS(android)
- **low: 45**（含已 RESOLVED 的 multirule）
- **total: 52**（含 1 个 RESOLVED 条目）

---

## 6. 能力对标真实状态（基于 97 项清单 + 批量测试）

### 已验证通过
- source.import: 30/30 P0 源 L1 通过 ✅
- MultiRule CSS &&/||/%%: yodu 真实源全链路通过 ✅

### 批量测试暴露的真实缺口（按影响排序）

| 缺口 | 影响 | 占比 | 优先级 |
|------|------|------|--------|
| 规则补全 (RuleComplete) | no_search_results | 34% | P0 |
| URL JS 执行 (java.get/post host callback) | URL JS 执行失败 | 28% | P0 |
| 多页加载 (nextTocUrl/nextContentUrl) | L4/L5 全 skip | — | P0 |
| 编码检测 (GBK/GB2312) | 部分 no_search_results | ~5% | P1 |
| JSONPath MultiRule | invalid JSONPath | 3% | P1 |

### 完全未实现的 45 项能力（见 CAPABILITY_GAP_PLAN.md）

---

## 7. 测试工具链状态

| 组件 | 状态 |
|------|------|
| corpus-manager (import/classify) | ✅ 459 源已导入分类 |
| reader-cli --test-source | ⚠️ WIP (1020行，有编译问题) |
| reader-cli --test-corpus | ⚠️ WIP (388行，有编译问题) |
| CLI HTTP 客户端 (ureq) | ⚠️ WIP (Cargo.toml 已加依赖) |
| 链式提取 | ⚠️ WIP |
| corpus-batch-p0.json | ✅ 首批结果(30源 live) |
| corpus-batch-offline-test.json | ✅ 首批结果(30源 offline) |
| recorded/ 录像 | ⚠️ 仅 1 个源 |
| CI workflow | ⚠️ WIP (core.yml +231行) |

---

## 8. 关键缺口清单（基于批量测试真实数据修正优先级）

### P0-blocker（阻断 L2-L5，影响最大）

1. **规则补全 (RuleComplete)** — 34% 源因 no_search_results 失败
   - Legado RuleComplete.kt autoComplete()，省略尾操作符的规则返回空
2. **URL JS 执行 (java.get/post)** — 28% 源因 @js: searchUrl 失败
   - JS sandbox 中 java.get/post 等 host callback 未接通
   - 这不是 reader-js 的问题（79 个方法有单元测试），是 runtime 中
     JS → host callback → http.execute 的链路未通
3. **多页加载 (nextTocUrl/nextContentUrl)** — L4/L5 无法验证
   - 不补则目录和正文只能取第一页

### P1-core（Legado 核心能力）

4. 替换规则 (ReplaceRule)
5. 繁简转换 (t2s/s2t)
6. TXT 目录规则 (TxtTocRule)
7. 书签 (Bookmark)
8. 书架分组 (BookGroup)
9. 阅读记录 (ReadRecord)
10. 发现 (Explore)

### P2（重要能力）

11. 段评 / 字体反混淆 / 封面解密 / 全文搜索 / 换源 / 去重标题 / 智能分段

---

## 9. 平台侧状态

### Android (codex/android-real-core-runtime-evidence)
- CoreEndToEndTest PASS: search→detail→toc→content 全链路 (simulator)
- CoreSmokeTest 3/3 PASS
- InstrumentedSmokeTest 4/4 PASS
- 缺失: ForegroundService、SAF 入口、TextToSpeech 发声

### iOS (codex/ios-real-app-core-evidence)
- S6.2 完成: default ServiceMode .rustCore
- 110/110 smoke test pass
- 缺失: 签名配置、BGTaskScheduler、完整登录流

### HarmonyOS (codex/harmony-signed-device-runtime)
- Task 1-8 完成: Index.d.ts + host/ 模块 + api/ facade + UI 切换
- BUILD SUCCESSFUL
- 缺失: 主题系统、device proof、签名配置

---

*本文为 2026-06-27 时间点快照（第三次更新）。后续工作必须以实际代码验证为准。*
*批量测试通过率 0% 是真实数据，不是猜测——30 个 P0 源 live 测试，L2-search 仅 1 通过。*
