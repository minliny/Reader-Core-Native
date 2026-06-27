# 项目状态快照

扫描日期：2026-06-27
本文为时间点快照，不作为永久事实。后续工作必须以实际代码验证为准。

---

## 1. 仓库状态

| 仓库 | 分支 | 最新 commit | 状态 |
|------|------|-------------|------|
| Core (Reader-Core-Native) | main | 62e6765d | 有 MultiRule agent 未提交变更(reader-rule + reader-content)，不动 |
| Android (Reader for Android) | codex/android-real-core-runtime-evidence | 19dfdab | 已 push，8/8 instrumented test pass (simulator) |
| iOS (Reader for iOS) | codex/ios-real-app-core-evidence | a845066 | 已 push，110/110 smoke test pass |
| HarmonyOS (Reader for HarmonyOS) | codex/harmony-signed-device-runtime | 338205f | host/ 11文件 + api/ 4文件，BUILD SUCCESSFUL |
| Reader-Core (Swift 旧 Core) | codex/atcss-prefix-booklist-fix | 7eac5058 | 冻结 |
| Reader UI | codex/motion-demo-optimizations | 1a635ec | 设计稿 + 前端 demo |

---

## 2. 已完成 Agent 结果记录

### Agent A — 审计 8 个 medium blocker（commit dd824aca）

- 删除 2 个已修 HarmonyOS blocker：
  - rb-harmonyos-index-dts-stale → commit 412d08c 已修
  - rb-harmonyos-host-adapter-missing → host/ 模块已建(10 文件真实实现)
- 保留 6 个 medium blocker，reason/mitigation 已刷新：
  - rb-包体签名和分发 (all) — 三平台均未实现签名配置
  - rb-后台任务和通知 (all) — Android 仅 Notification 无 ForegroundService
  - rb-登录-webview-交互 (all) — 三平台均无完整登录流
  - rb-主题和字体-harmony (harmony) — 硬编码，无主题系统
  - rb-文件选择和沙箱授权-android (android) — adapter ready，无 SAF 入口
  - rb-系统-tts-发声-android (android) — 仅 Fake TTS
- Summary 修正：51 total / 6 medium / 44 low / 1 blocker
- conformance 回归：173 passed / 0 failed

### Agent B — HarmonyOS S6 Task 3/4/5（commit 0c73bc2 + 338205f）

- Task 3: host/ 模块 11 文件 767 行（HostTransport/HostAdapter/HostBus/HostRuntime/
  HostCommander/HttpExecuteHandler/HttpRequest/HostReply/HostReplyCodec/CapabilityHandler）
- Task 4: OHOSHTTPHostTransport.ets（@ohos.net.http 实现 HostTransport）
- Task 5: api/ 模块 4 文件 499 行（ReaderCoreClient/BookApi/SourceApi/BookshelfApi）
- 验证：hvigorw assembleHap BUILD SUCCESSFUL
- 证据级别：Build proof（非 device proof）

### Agent C — HarmonyOS S6 Task 6/7/8（commit 0c29184）

- Task 6: 退役 fixture 业务路径（strangler 模式，标记 deprecated 不删除）
- Task 7: 5 个 UI 页面切换到 Rust Core（Bookshelf/Search/Reader/Settings/ImportBookSource）
- Task 8: E2E 测试需设备/模拟器（本地无 hdc target）
- 证据级别：Build proof（非 device proof）

### Agent D — MultiRule 拆分（未完成，agent 仍在运行）

- 工作树有未提交变更：reader-rule/src/lib.rs (+391 行) + reader-content/src/lib.rs (+35 行)
- 新增测试文件：legado_multirule_operator.rs（15 tests pass）
- reader-rule 全部测试通过：130 unit + 15 multirule = 145 tests pass
- 影响：459 源中 292 源含 MultiRule（64%），修复后预期大幅提升批量测试通过率
- 状态：仍在运行，不触碰

---

## 3. S 阶段进度（修正后）

| 阶段 | 之前声称 | 修正后实际 | 依据 |
|------|---------|-----------|------|
| S0 (baseline) | 100% | ✅ 100% | — |
| S1 (BookSource compat) | 100% | ⚠️ ~40% | 97 项能力中仅 22 项实现，459 源从未批量测试 |
| S2 (DSL) | 70% | ⚠️ ~50% | MultiRule blocker 未闭环(正在修)，JS 端到端未验证 |
| S3 (request/JS/host) | 100% | ⚠️ ~60% | 79 个 java.* 方法有单元测试，但真实源 JS 执行未验证 |
| S4 (remote reading) | 60% | ⚠️ ~30% | 3 源 fixture (0.65%)，多页加载/规则补全未实现 |
| S5 (data/local/RSS/sync) | 90% | ⚠️ ~50% | crate test 存在但替换规则/繁简/TXT目录/书签等未实现 |
| S6 (strangler migration) | 45% | ⚠️ ~50% | Android simulator proof ✅；iOS smoke 110/110 ✅；HarmonyOS build proof ✅ |
| S7 (corpus benchmark) | 0% | ❌ 0% | 工具存在但从未跑过 |

---

## 4. Release Blockers 状态

- **blocker (1):** rb-legado-css-multirule-operator — Agent 正在修，15 tests pass
- **medium (6):** 签名/后台任务/登录(all) + 主题(harmony) + SAF(android) + TTS(android)
- **low (44):** 各种能力 blocker，多为"有实现但未用真实源验证"
- **total: 51**

---

## 5. Legado 能力对标真实状态

基于 docs/LEGADO_CAPABILITY_INVENTORY.md（97 项能力审计）：

| 状态 | 数量 | 说明 |
|------|------|------|
| 已实现 | 22 | 大部分只有单元测试，无真实源端到端验证 |
| 部分实现 | 16 | 从未用真实 Legado 数据验证 |
| 未实现 | 45 | 替换规则/繁简/TXT目录/书签/书架分组/段评/多页/发现/字体反混淆/封面解密/全文搜索/换源等 |
| Host/UI 层 | 14 | 不在 Core 范围 |
| **合计** | **97** | |

**Core 侧实际完成度：~23%**（22/83 非Host能力，且大部分无真实源验证）

---

## 6. 测试工具链状态

| 组件 | 状态 |
|------|------|
| corpus-manager (import/classify) | ✅ 已创建，459 源已导入分类 |
| reader-cli --corpus-import-batch | ❌ 未实现 |
| reader-cli --test-source (单源 L1-L5) | ❌ 未实现（缺 HTTP 客户端 + 链式提取） |
| reader-cli --test-corpus (批量) | ❌ 未实现 |
| 17 个 Python 工具 | ❌ 全部未跑过 |
| CI (core.yml) | ⚠️ 仅 fmt+test+conformance |
| 459 源批量通过率 | ❓ 未知（从未测试） |

**关键断链：** CLI 没有 HTTP 客户端（0 行 HTTP 代码），无法发真实请求。
现有 fixture-vertical 需要一次性喂全部 mock 响应，不链式提取。
无法实现"给一个书源就跑完所有测试"。

---

## 7. 关键缺口清单

### 阻断真实书源（P0-blocker）
1. MultiRule 拆分 — Agent 正在修（292/459 源受影响）
2. 多页加载 (nextTocUrl/nextContentUrl) — 完全未实现
3. 规则补全 (RuleComplete) — 完全未实现

### Legado 核心能力缺失（P1-core）
4. 替换规则 (ReplaceRule) — 完全未实现
5. 繁简转换 (t2s/s2t) — 完全未实现
6. TXT 目录规则 (TxtTocRule) — 完全未实现
7. 书签 (Bookmark) — 完全未实现
8. 书架分组 (BookGroup) — 完全未实现
9. 阅读记录 (ReadRecord) — 完全未实现
10. 发现 (Explore) — 有字段无协议方法

### 重要能力缺失（P2）
11. 段评 (ReviewRule)
12. 字体反混淆 (QueryTTF)
13. 封面解密 (coverDecodeJs)
14. 全文搜索 (SearchContent)
15. 换源 (ChangeSource)
16. 去重标题
17. 智能分段

### 测试基础设施缺失
18. CLI HTTP 客户端（ureq/reqwest）
19. 链式提取逻辑（search→detail→toc→content URL 传递）
20. --test-source / --test-corpus 命令
21. 录像 + 离线回放
22. CI 四级 gate

---

## 8. 平台侧状态

### Android (codex/android-real-core-runtime-evidence)
- libreader_core.a 重建 (core main@81679ea5)
- CoreEndToEndTest PASS: search→detail→toc→content 全链路 (simulator, 121.897s)
- CoreSmokeTest 3/3 PASS: JNI 链接 + abiVersion=1 + pingSmoke
- InstrumentedSmokeTest 4/4 PASS: keystore/SAF/WebView/cookie
- 缺失: ForegroundService、SAF 入口、TextToSpeech 发声

### iOS (codex/ios-real-app-core-evidence)
- S6.2 完成: default ServiceMode .rustCore + fail-loud
- 110/110 smoke test pass
- RustCoreBookDetailService 已接入
- 缺失: 签名配置、BGTaskScheduler、完整登录流

### HarmonyOS (codex/harmony-signed-device-runtime)
- Task 1: Index.d.ts 对齐 reader_napi.cpp 12 导出 ✅
- Task 3/4/5: host/ 模块 + OHOSHTTPHostTransport + api/ facade ✅ (build proof)
- Task 6/7/8: 退役 fixture + 5 页 UI 切换 ✅ (build proof)
- 缺失: 主题系统、device proof (无 hdc target)、签名配置

---

*本文为 2026-06-27 时间点快照。后续工作必须以实际代码验证为准，
不得凭此快照声称能力已完成。*
