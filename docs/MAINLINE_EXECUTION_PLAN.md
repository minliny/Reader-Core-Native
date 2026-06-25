# 主线执行计划

日期：2026-06-25

本文是 `docs/LOCAL_REPO_MIGRATION_DIRECTIVE.md` 的落地执行计划。若本文件与历史
roadmap、旧审计报告、单个 agent 总结冲突，以 `LOCAL_REPO_MIGRATION_DIRECTIVE.md`
和本文为准。

## 1. 主线不变量

Reader 迁移只有一条主线：

```text
Legado 定义要兼容什么
  -> 旧 Reader-Core 定义已有能力如何迁移
  -> Reader-Core-Native 用 Rust Core + C ABI 定义全平台接入边界
  -> corpus benchmark 证明 CLI / iOS / Android / HarmonyOS 读出同样结果
```

任何开发分支都必须能回答：

1. 本轮兼容目标来自本地 `legado` 的哪个代码路径或数据结构。
2. 本轮迁移资产来自本地 `Reader-Core` 的哪个代码路径、测试或 sample。
3. 本轮 Rust 改动落在哪个 crate、protocol schema、C ABI 或 binding。
4. 本轮是否改变三端 host adapter 的责任边界。
5. 本轮证据是 crate test、CLI conformance、FFI smoke、wrapper smoke、App/device proof
   还是 corpus benchmark。

不能回答这些问题的工作不能合入主线。

## 2. 当前事实快照

最近一次本地扫描：2026-06-25。后续 agent 仍必须重新扫描，本表只记录本 checkpoint。

| 仓库 | 当前角色 | 当前分支 / 状态 |
| --- | --- | --- |
| `/Users/minliny/Documents/Reader-Core-Native` | Rust 目标 Core | `main` 当前为 `fc5fb57`；checkpoint、BookSource compat、DSL executor、data fixture gates、corpus tools、Android/iOS host evidence 已进入主线 |
| `/Users/minliny/Documents/legado` | 兼容语义基线 | `master`，只读 |
| `/Users/minliny/Documents/Reader-Core` | 旧 Core 迁移源 | `main`，只读迁移参考 |
| `/Users/minliny/Documents/Reader for iOS` | iOS host | `codex/ios-rust-host-adapter` |
| `/Users/minliny/Documents/Reader for Android` | Android host | `main` |
| `/Users/minliny/Documents/Reader for HarmonyOS` | HarmonyOS host | `codex/harmony-napi-runtime` |

当前 PR / 分支队列：

- PR #3 / #4 / #15 / #14 / #13 / #2 / #12 已按顺序合入 `main`。
- HarmonyOS PR #2 继续保持 draft；已有 headless/simulator/package 证据，但没有 real-device proof。
- PR #16 `codex/reader-js-compat-runtime` 已打开，范围限定 `crates/reader-js/**`。
- 旧工具 PR #5-#11 中未合入的分支仍只能表示工具候选，不表示 release gate 或 corpus benchmark 完成。

## 3. 强制阶段顺序

### S0：主线基线与工作区卫生

目标：保证所有后续 agent 不再从旧路径、旧分支、旧 dirty 状态开始。

写入范围：

- `docs/LOCAL_REPO_MIGRATION_DIRECTIVE.md`
- `docs/MAINLINE_EXECUTION_PLAN.md`
- `docs/FULL_DEVELOPMENT_ROADMAP.md`
- `docs/ROLLING_INTEGRATION.md`
- `MIGRATION_MAP.md`

退出条件：

- 文档中没有把 `Reader-Core-Rust` 当作当前本机目标仓库。
- 文档中所有“当前状态”都标注为某次扫描快照，而非永久事实。
- 所有关联仓库 dirty 状态已审计并说明。
- `git diff --check` 通过。

### S1：Legado BookSource 兼容基础

目标：先兼容 Legado 书源结构，避免继续把 BookSource 规则硬压成 `RuleStepSpec`。

事实来源：

- `legado/app/src/main/java/io/legado/app/data/entities/BookSource.kt`
- `legado/app/src/main/java/io/legado/app/data/entities/rule/*.kt`
- `Reader-Core/Sources/ReaderCoreModels/BookSource.swift`
- `Reader-Core/samples/**/booksource.json`
- `Reader-Core/Tests/ReaderCoreModelsTests/BookSourceDecodingTests.swift`

Rust 写入范围：

- `crates/reader-domain`
- `crates/reader-contract`
- `crates/reader-runtime`
- `protocol/reader-command.schema.json`
- `protocol/fixtures/conformance/commands/*source-import*`
- `tools/reader-cli/src/conformance.rs`

必须保持：

- `LegadoBookSource` / `BookSourceCompat` 是兼容载荷模型，不是执行模型。
- `source.import.params.bookSource` 保存 raw Legado BookSource object。
- `Source.rules` 继续只表示当前 V1 结构化执行规则。
- raw Legado DSL 字符串不能被 `RuleStepSpec` 接受。

禁止：

- 禁止为了执行 Legado DSL 而扩展 `RuleStepSpec` 到可以接收
  `"div.list&&div.item;div.name&&a@text"` 这种字符串。
- 禁止把 `bookSource` 保存成功声明为“已经能读 Legado 书源”。
- 禁止复制 Legado GPL 实现代码；只能做本地代码行为审计和 clean-room Rust 实现。

退出条件：

- BookSource decode / encode roundtrip 保留 raw rule、unknown fields、header、nested rule。
- `source.import` 能接收并保存 `bookSource`。
- CLI conformance 覆盖 valid / invalid `bookSource`。
- `cargo test -p reader-domain -p reader-contract -p reader-runtime -p reader-cli`
  通过。
- `cargo run -p reader-cli -- --conformance` 通过。

当前快照：S1 已通过 PR #4 进入 `main`。后续只能继续扩 Legado 字段、样本和异常
fixture，不能把保存 raw `bookSource` 误写成完整执行能力。

### S2：Legado DSL parser / executor

目标：独立实现 Legado CSS 管道链 DSL，不再借 `RuleStepSpec` 规避语义差异。

建议写入范围：

- `crates/reader-rule`：DSL tokenizer、AST、selector/extractor primitive。
- `crates/reader-content`：`LegadoBookSource` 到 search/detail/toc/content pipeline 的 adapter。
- `fixtures/` 或 `protocol/fixtures/`：sanitized BookSource + response + expected output。
- `tools/reader-cli`：最小 Legado fixture runner 或现有 `--fixture-vertical` 扩展。

核心对象建议：

- `LegadoRuleDsl`
- `LegadoRuleAst`
- `LegadoRulePipeline`
- `LegadoRuleExtractor`
- `LegadoBookSourceAdapter`

第一批只做最小闭环：

1. `searchUrl` 模板：`{{key}}`、`{{page}}`、基础 query 编码。
2. `ruleSearch` / `searchRule.bookList`。
3. `searchRule.name`、`searchRule.author`、`searchRule.bookUrl`。
4. `ruleBookInfo` / `bookInfoRule.name`、`author`、`tocUrl`。
5. `ruleToc` / `tocRule.chapterList`、`chapterName`、`chapterUrl`。
6. `ruleContent` / `contentRule.content`。

第一批必须支持的 DSL 形态：

- `selector@text`
- `selector@html`
- `selector@href`
- `a&&@href`
- `div.list&&div.item`
- `div.list&&div.item;div.name&&a@text`
- 空规则、缺失规则、无匹配结果的 fail-closed 行为。

延期项：

- WebView 登录、captcha、DOM 执行。
- 真实 HTTP socket。
- 完整 JS bridge。
- 文件、字体、压缩包、native command。

退出条件：

- 至少 1 个旧 Reader-Core sample 被迁移为 sanitized Rust fixture。
- CLI 能跑 import -> search -> detail -> toc -> content。
- 同一 fixture 有 canonical expected JSON。
- `RuleStepSpec` guard 测试仍通过。

当前快照：S2 最小 DSL executor 已通过 PR #15 进入 `main`。已建立 `reader-rule`
DSL 路径和 `reader-content` raw string stage 执行路径；仍未覆盖完整 Legado 规则语言、
WebView/DOM/登录或真实网络。

### S3：请求描述、JS 与 host capability 契约

目标：把旧 Core 的 request descriptor、JS helper、Cookie/session/redirect 语义拆成
Core-owned descriptor 与 host-owned execution。

Core owns：

- URL 模板展开。
- method / headers / body / charset / retry / redirect policy descriptor。
- cookie/session handle 的协议字段。
- JS helper 中不依赖平台的纯函数。
- host request / host complete / host error 的 schema、错误码、conformance。

Host owns：

- socket / TLS / HTTP stack。
- Cookie jar 实体存储。
- WebView 登录、captcha、DOM。
- Keychain / Keystore / credential store。

退出条件：

- `http.execute` params/result/error schema 覆盖 request descriptor 所需字段。
- CLI host replay 能回放固定 HTTP/cookie/redirect fixture。
- iOS / Android / HarmonyOS adapter 各有同一 contract 的 smoke 证据。
- 不把 host app 能力写成 Core 已完成。

当前快照：JS lane 已从 dirty/stash 中整理为 PR #16 `codex/reader-js-compat-runtime`，
只证明 `reader-js` helper/runtime 边界；request descriptor 与 host capability 扩展仍未完成。

### S4：Remote reading end-to-end

目标：用 Legado BookSource + DSL + host replay 把远程阅读链路跑通。

范围：

- `source.import`
- `book.search`
- `book.detail`
- `book.toc`
- `chapter.content`
- `reading.progress.update`

退出条件：

- CLI 端到端 fixture 通过。
- 结果进入 canonical DTO。
- 失败路径有结构化错误和可复现 fixture。
- 不需要真实网络也能回放授权样本。

### S5：Data / local book / RSS / sync

目标：Rust Core 统一数据语义，平台只提供目录、权限和系统服务。

Core owns：

- SQLite schema/migration 或等价持久化模型。
- bookshelf / progress / history / cache / download queue 语义。
- TXT / EPUB / RSS 数据解析。
- WebDAV package、conflict、diff、recovery。

Host owns：

- 文件 picker。
- sandbox / SAF / Harmony 文件授权。
- WebDAV HTTP transport。
- TTS 播放。

退出条件：

- snapshot/import/export fixture 通过。
- 同一数据 fixture 可生成 canonical hash。
- 本地书/RSS/WebDAV 不依赖平台 UI 才能验证核心语义。

当前快照：storage migration/snapshot、canonical hash、TXT/EPUB fixture gates 已通过
PR #14 进入 `main`。RSS/WebDAV/sync/TTS 仍需后续分支补齐。

### S6：三端 strangler migration

目标：三端都通过 C ABI / JSON protocol 使用同一个 Rust Core commit。

iOS：

- Swift wrapper 创建/销毁 runtime。
- URLSession host transport。
- WKWebView 登录/cookie/captcha 作为 host capability。

Android：

- JNI/Kotlin wrapper 创建/销毁 runtime。
- OkHttp host transport。
- WebView/CookieManager/Keystore/SAF 作为 host capability。

HarmonyOS：

- Node-API/ArkTS wrapper 创建/销毁 runtime。
- Harmony HTTP/WebView/credential/file 作为 host capability。
- HAP/device evidence 与 simulator evidence 必须分层标注。

退出条件：

- 三端都能 create/send/event/cancel/destroy。
- 三端都能处理 `http.execute` host request。
- 三端最小阅读链路 smoke 使用同一 Rust Core commit。
- wrapper smoke、simulator smoke、device proof 分开报告。

当前快照：Android Native JVM host adapter 已通过 PR #2 进入 `main`；iOS Native
`bindings/ios` shell smoke 已通过 PR #12 进入 `main`；HarmonyOS host PR #2 仍是
draft，缺 real-device proof。三端均未完成真实 App/device 阅读链路。

### S7：Corpus benchmark / release gate

目标：证明“三端真的读出同样结果”。

工具入口：

- 已进入主线：PR #13 的 `scripts/corpus_canonicalize.py`、
  `tools/cross-platform-diff/`、`tools/benchmark-run-packager/`、
  `tools/release-blocker-register/` 和 sample/demo 文档。
- 仍可独立评审：ABI symbol checker、host request replay、reader text normalization、
  evidence fixture tooling 等旧工具分支。

退出条件：

- CLI / iOS / Android / HarmonyOS 对同一 corpus 输出 canonical DTO。
- cross-platform diff 为零或差异有 approved waiver。
- release blocker register 无 P0/P1 未关闭项。
- benchmark packager 归档 commit、平台、命令、输出 hash。

## 4. 分支边界

| 分支/PR | 目标 | 允许修改 | 禁止修改 | 合入前证据 |
| --- | --- | --- | --- | --- |
| PR #3 `codex/full-branch-directory-consolidation` | checkpoint base | docs、已整合基础代码 | 新增未验证能力声明 | 已合入 |
| PR #4 `codex/booksource-compat-protocol` | BookSource 兼容入口 | domain/contract/runtime/protocol/CLI fixture | 用 `RuleStepSpec` 执行 raw Legado DSL | 已合入 |
| PR #15 `codex/legado-rule-dsl-executor` | Legado DSL 执行 | `reader-rule`、`reader-content`、fixtures、CLI runner | 改 C ABI、平台 binding、真实网络 | 已合入 |
| PR #16 `codex/reader-js-compat-runtime` | JS helper/runtime 兼容 | `crates/reader-js/**` | 改 ABI/protocol/storage/bindings，或实现真实网络/WebView | `cargo test -p reader-js`、fmt、diff check |
| `codex/request-host-contract` | request / host capability | contract/runtime/protocol/CLI replay | host app UI、真实 socket | conformance、host replay |
| PR #2 / PR #12 / Harmony host PR #2 | 三端 adapter | 对应 binding 或 host app | Core 业务语义 | Android/iOS 已合入 shell/JVM 证据；Harmony draft，缺 real-device |
| PR #13 / 工具分支 | benchmark/release 工具 | tools/scripts/tests/samples | 宣称 release gate 完成 | PR #13 已合入；其他工具分支按需评审 |

## 5. 防偏离规则

### 5.1 RuleStepSpec 与 Legado DSL

`RuleStepSpec` 是 V1 结构化规则执行格式。Legado DSL 是另一套兼容语言。

必须保持：

- raw Legado DSL 字符串保留在 `LegadoBookSource` / `bookSource`。
- Legado DSL 只能进入专门的 `LegadoRuleDsl` / `LegadoRulePipeline`。
- `RuleStepSpec` 只能继续接收 `{ "kind": ... }` 结构化对象。

禁止：

- 让 `RuleStepSpec` 反序列化裸字符串。
- 把 `selector@text`、`a&&@href`、`;` 链拆成临时 JSON step 后宣称 Legado 兼容完成。
- 只凭 parser 单测宣称远程阅读链路完成。

### 5.2 Core 与 host app

Core 不开 socket，不直接使用 WebView，不保存明文凭据。

必须保持：

- Core 产出 request descriptor。
- Host 执行 `http.execute`、WebView、credential、file、TTS。
- Host completion 通过 JSON protocol 返回 Core。

禁止：

- 在 Core 内引入平台 HTTP 客户端。
- 把 App/设备能力写成 Core 已完成。
- 把 wrapper smoke 当作 App/device proof。

### 5.3 Benchmark 与 release

工具存在不等于 benchmark 完成。

必须保持：

- canonical DTO schema 固定。
- CLI / iOS / Android / HarmonyOS 均使用同一 Rust Core commit。
- 差异必须进入 diff report 和 release blocker register。

禁止：

- 只用 CLI 单端结果宣称三端一致。
- 只用静态报告宣称 release ready。
- 只用 simulator 结果宣称 real device parity。

## 6. 每轮开发模板

每轮开始前写入提交说明或 PR 描述：

```text
主线阶段：
事实来源：
- Legado:
- Reader-Core:
- Host app:
写入范围：
明确不做：
验证命令：
可运行结果：
是否影响 ABI/protocol:
是否需要三端 adapter 跟进:
是否能进入 corpus benchmark:
```

每轮结束必须给出：

- 修改文件清单。
- 测试/构建命令和结果。
- 仍未验证的层级。
- 是否存在 host app 后续任务。
- 是否存在 release blocker。

## 7. 当前下一步

下一步不是继续扩 `RuleStepSpec`，也不是把 wrapper smoke 当作三端完成。当前主线应按
以下顺序推进：

```text
1. 评审并合入 PR #16 `codex/reader-js-compat-runtime`：只收敛 JS helper/runtime，保持 host-owned 能力边界。
2. 新开或恢复 `codex/request-host-contract`：补 `HostHttpRequest` 的 charset/cookie/redirect/retry、
   fixture、CLI host replay、conformance。
3. 在 HarmonyOS PR #2 继续补 real-device tier；无设备时保持 draft，不声明完成。
4. 用已合入的 corpus 工具跑 CLI + iOS + Android + HarmonyOS 同一 corpus；差异进入
   release blocker register，不能用单端结果替代三端 parity。
```

上述事项完成后，才能进入远程阅读链路和 release gate 的完成判定。
