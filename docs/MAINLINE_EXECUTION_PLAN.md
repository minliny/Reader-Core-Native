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
| `/Users/minliny/Documents/Reader-Core-Native` | Rust 目标 Core | `codex/full-branch-directory-consolidation` 为 checkpoint base；BookSource 纠偏在 `codex/booksource-compat-protocol` / PR #4 |
| `/Users/minliny/Documents/legado` | 兼容语义基线 | `master`，只读 |
| `/Users/minliny/Documents/Reader-Core` | 旧 Core 迁移源 | `main`，只读迁移参考 |
| `/Users/minliny/Documents/Reader for iOS` | iOS host | `codex/ios-rust-host-adapter` |
| `/Users/minliny/Documents/Reader for Android` | Android host | `main` |
| `/Users/minliny/Documents/Reader for HarmonyOS` | HarmonyOS host | `codex/harmony-napi-runtime` |

PR 队列原则：

- PR #3：Native checkpoint base，对 `main`。
- PR #4：BookSource 兼容纠偏，对 PR #3。
- PR #1 / #2：iOS / Android host adapter，对 PR #3。
- PR #5-#11：benchmark / evidence / replay / normalization 工具，对 PR #3。
- 工具 PR 只表示工具入队，不表示 release gate 或 corpus benchmark 完成。

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

### S7：Corpus benchmark / release gate

目标：证明“三端真的读出同样结果”。

工具入口：

- PR #5：ABI symbol checker。
- PR #6：benchmark run packager。
- PR #7：corpus canonicalizer。
- PR #8：cross-platform result diff。
- PR #9：evidence fixture tooling / release blocker registry。
- PR #10：host request replay。
- PR #11：reader text normalization。

退出条件：

- CLI / iOS / Android / HarmonyOS 对同一 corpus 输出 canonical DTO。
- cross-platform diff 为零或差异有 approved waiver。
- release blocker register 无 P0/P1 未关闭项。
- benchmark packager 归档 commit、平台、命令、输出 hash。

## 4. 分支边界

| 分支/PR | 目标 | 允许修改 | 禁止修改 | 合入前证据 |
| --- | --- | --- | --- | --- |
| PR #3 `codex/full-branch-directory-consolidation` | checkpoint base | docs、已整合基础代码 | 新增未验证能力声明 | `cargo test --workspace`、conformance、FFI smoke |
| PR #4 `codex/booksource-compat-protocol` | BookSource 兼容入口 | domain/contract/runtime/protocol/CLI fixture | 用 `RuleStepSpec` 执行 raw Legado DSL | BookSource roundtrip、conformance |
| `codex/legado-rule-dsl-executor` | Legado DSL 执行 | `reader-rule`、`reader-content`、fixtures、CLI runner | 改 C ABI、平台 binding、真实网络 | DSL parser/executor tests、CLI fixture |
| `codex/request-js-host-contract` | request / JS / host capability | contract/runtime/js/content/protocol/CLI | host app UI、真实 socket | conformance、host replay |
| PR #1 / #2 / Harmony host PR | 三端 adapter | 对应 binding 或 host app | Core 业务语义 | wrapper/app/device 分层证据 |
| PR #5-#11 | benchmark/release 工具 | tools/scripts/tests/samples | 宣称 release gate 完成 | tool tests |

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

下一步不是继续扩 `RuleStepSpec`，而是新开或继续一个明确的 Legado DSL executor 分支：

```text
目标：实现 Legado CSS 管道链 DSL 的最小 search/detail/toc/content 闭环。
事实来源：本地 legado BookSource.kt/rule/*.kt、本地 Reader-Core BookSource.swift、旧 Core samples。
写入范围：crates/reader-rule、crates/reader-content、fixtures、tools/reader-cli。
禁止：改 C ABI、改三端 binding、让 RuleStepSpec 接收 raw DSL 字符串、声明 App/device 完成。
验证：cargo test -p reader-rule -p reader-content；cargo run -p reader-cli -- --fixture-vertical <legado-sanitized-fixture>。
```

只有这个阶段完成后，三端 adapter 和 corpus benchmark 才有可执行的核心能力可以消费。
