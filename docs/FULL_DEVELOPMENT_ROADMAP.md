# Reader Rust Core 全量开发路线

日期：2026-06-25

最高优先级入口：`docs/LOCAL_REPO_MIGRATION_DIRECTIVE.md`

主线执行计划：`docs/MAINLINE_EXECUTION_PLAN.md`

当前 Rust 目标仓库：`/Users/minliny/Documents/Reader-Core-Native`

说明：本轮文档和开发统一以 `Reader-Core-Native` 为 Rust 目标仓库。后续任何 agent
仍必须先扫描本地工作区，确认目标仓库路径、Git 状态和分支后再开始修改。
若本文的阶段描述与 `docs/MAINLINE_EXECUTION_PLAN.md` 的执行顺序或禁止项冲突，
以 `docs/MAINLINE_EXECUTION_PLAN.md` 为准。

## 最终目标

把现有 Reader 项目迁移为三端共用同一个 Rust Reader-Core 的架构：

- iOS 不再以 Swift 旧 Core 承担业务内核职责。
- Android 不再保留独立业务核心实现。
- HarmonyOS 不再保留独立业务核心实现。
- Rust Reader-Core 通过 C ABI 被 Swift、JNI、Node-API/ArkTS 消费。
- 三端只保留平台能力、UI、权限、WebView、系统服务、打包分发和平台生命周期。

迁移完成的判定是：iOS、Android、HarmonyOS 运行同一个 Rust Core commit，对同一组
本地 fixture/corpus 得到一致的书源、目录、章节内容、进度、缓存、同步结果。

## 本地事实来源

本地仓库是唯一事实来源：

| 角色 | 默认目录 | 当前本机定位 | 用途 |
| --- | --- | --- | --- |
| 旧业务核心 | `Reader-Core` | `/Users/minliny/Documents/Reader-Core` | 迁移源、行为参考、测试/fixture 来源 |
| iOS 宿主 | `Reader-for-iOS` | `/Users/minliny/Documents/Reader for iOS` | SwiftUI App、Swift wrapper、URLSession/WebView/Keychain/File/TTS 等平台 adapter |
| Android 宿主 | `Reader-for-Android` | `/Users/minliny/Documents/Reader for Android` | Android App、Kotlin/JNI、OkHttp/WebView/Keystore/Room/系统服务 |
| HarmonyOS 宿主 | `Reader-for-HarmonyOS` | `/Users/minliny/Documents/Reader for HarmonyOS` | ArkTS/HarmonyOS NEXT、Node-API、平台 adapter、HAP/设备验证 |
| Rust 目标 Core | `Reader-Core-Native` | `/Users/minliny/Documents/Reader-Core-Native` | 唯一业务内核、C ABI、跨平台 runtime |

远程 README、历史讨论、旧架构报告只能补充上下文，不能覆盖本地代码。

## 当前主线进度快照

最近一次快照：2026-06-25，`Reader-Core-Native` `origin/main` =
`fc5fb57`。

| 主线项 | 当前状态 |
| --- | --- |
| checkpoint base | PR #3 已合入。 |
| Legado BookSource raw object / unknown field / raw rule 保真 | PR #4 已合入；后续继续扩字段和样本，不代表完整执行能力。 |
| Legado CSS DSL executor | PR #15 已合入；`RuleStepSpec` 仍保持结构化 V1，不接收 raw DSL 字符串。 |
| JS helper/runtime 兼容 | PR #16 `codex/reader-js-compat-runtime` 已打开；只覆盖 `crates/reader-js/**`，不实现真实网络/WebView。 |
| request descriptor / host capability | 仍待 `codex/request-host-contract` 或后续分支扩展。 |
| storage/local-book fixture gates | PR #14 已合入；RSS/WebDAV/sync/TTS 仍待扩展。 |
| corpus release gate 工具基础 | PR #13 已合入；这只是工具基础，不是三端 benchmark 完成。 |
| Android Native host evidence | PR #2 已合入；JVM host adapter 证据不是 `.so`/AAR/设备 proof。 |
| iOS Native host evidence | PR #12 已合入；shell smoke 不是 iOS App/模拟器/真机 proof。 |
| HarmonyOS host evidence | HarmonyOS PR #2 保持 draft；已有 headless/simulator/package 证据，缺 real-device proof。 |

## 总体架构

```text
Reader-Core（旧核心）
  -> 行为审计
  -> 测试/fixture 迁移
  -> 能力差距清单

Rust Reader-Core（唯一业务内核）
  -> rule / JS / parser / request descriptor
  -> book source / search / detail / toc / content / progress
  -> local book / EPUB / RSS / WebDAV / sync / cache / diff
  -> SQLite model / migration / snapshot
  -> C ABI

iOS
  -> Swift wrapper
  -> URLSession / WKWebView / Keychain / file picker / TTS / UI

Android
  -> JNI / Kotlin wrapper
  -> OkHttp / WebView / Keystore / SAF / TTS / UI

HarmonyOS
  -> Node-API / ArkTS wrapper
  -> HTTP / WebView / credential store / file picker / TTS / UI
```

## 分阶段路线

### 不可跳过的主线顺序

```text
Legado 定义要兼容什么
  -> 旧 Reader-Core 定义已有能力如何迁移
  -> Reader-Core-Native 用 Rust Core + C ABI 定义全平台接入边界
  -> corpus benchmark 证明 CLI / iOS / Android / HarmonyOS 读出同样结果
```

这不是建议顺序，而是合入顺序。当前最重要的纠偏点是 BookSource / Rule：

- Legado `BookSource` 和 CSS 管道链 DSL 是兼容目标。
- 旧 `Reader-Core` 的 BookSource model、sample、parser 测试是迁移资产。
- `Reader-Core-Native` 的 `RuleStepSpec` 是 V1 结构化规则执行格式，不是 Legado DSL。
- raw Legado DSL 必须先保存在 `LegadoBookSource` / `BookSourceCompat`，再由独立
  `LegadoRuleDsl` / `LegadoRulePipeline` 执行。
- 禁止继续通过扩展 `RuleStepSpec` 来硬凑 `div.list&&div.item;div.name&&a@text`
  这类 Legado 字符串。

### 阶段 0：本地仓库定位与安全基线

目标：每次工作先确认本地仓库、分支、dirty 状态和最新提交。

必须执行：

```bash
pwd
find .. -maxdepth 2 -type d -name .git
git -C <repo> status --short
git -C <repo> branch --show-current
git -C <repo> log -5 --oneline
```

退出条件：

- 确认实际 Rust 目标仓库。
- 确认四个迁移源/宿主仓库状态。
- 不修改 unrelated dirty 文件。

### 阶段 1：旧 Reader-Core 代码审计

目标：从旧 `Reader-Core` 的实际代码提取迁移任务，而不是从历史文档猜测。

范围：

- 规则解析、CSS/XPath/JSONPath/Regex。
- JavaScript runtime / QuickJS / WebView 依赖边界。
- HTTP、Cookie、Session、Redirect、编码、重试。
- 书源导入、搜索、详情、目录、正文、分页。
- TXT/EPUB、本地书导入和阅读。
- RSS、WebDAV、同步、缓存、恢复、diff。
- 已有测试、fixture、sample report。

产出：

- `docs/migration/reader-core-code-audit.md`
- `docs/migration/reader-core-to-rust-task-map.md`
- 每个能力标记为：迁移到 Rust、保留为平台 adapter、废弃、延后。

### 阶段 2：Rust Core 基础契约冻结

目标：稳定三端共同消费的 Core 边界。

范围：

- C ABI lifecycle：create/send/cancel/destroy/version/error。
- JSON command/event protocol。
- runtime config。
- host operation bus。
- error taxonomy。
- C/C++ smoke。

退出条件：

- `cargo test --workspace`
- `cargo run -p reader-cli -- --conformance`
- `./scripts/ffi-smoke.sh`
- Swift/JNI/Node-API wrapper 能消费同一个 ABI 版本。

### 阶段 3：核心阅读能力迁移

目标：把旧 Core 中真正的业务能力迁到 Rust。

优先级：

1. BookSource 兼容入口：Legado raw object decode/encode、unknown field preserve、
   raw rule preserve、`source.import` conformance。
2. Legado DSL 执行器：CSS 管道链 DSL tokenizer/AST/pipeline/extractor，最小
   search -> detail -> toc -> content fixture 闭环。
3. Rule engine V1：保持 `RuleStepSpec` 结构化格式，用于已定义的 V1 JSONPath/CSS
   step，不接收 raw Legado DSL 字符串。
4. JS runtime：可嵌入 JS、host callback、timeout、cancel、安全边界。
5. Request descriptor：method、headers、body、charset、cookie、redirect、retry。
6. Remote reading：source import -> search -> detail -> toc -> content -> progress。
7. Local book：TXT、EPUB、章节切分、资源读取、编码检测。
8. Storage：SQLite schema、cache、progress、bookshelf、history、download queue。
9. Sync：WebDAV、backup/restore、conflict、diff。
10. RSS/TTS：Core 数据和契约，平台执行保留在 host。

退出条件：

- Rust crate 测试覆盖迁移能力。
- 旧 Core 对应测试或 fixture 被迁移/回放。
- CLI 可运行端到端 fixture。

### 阶段 4：三端 strangler migration

目标：让三端逐步把业务调用切到 Rust Core，保留平台 adapter。

iOS：

- Swift wrapper 消费 C ABI。
- URLSession 执行 `http.execute`。
- WKWebView 只负责登录/cookie/captcha/DOM-host 能力。
- Keychain、文件权限、TTS、UI 留在 iOS App。

Android：

- JNI/Kotlin wrapper 消费 C ABI。
- OkHttp 执行 `http.execute`。
- WebView/CookieManager/Keystore/SAF/TTS/UI 留在 Android App。

HarmonyOS：

- Node-API/ArkTS wrapper 消费 C ABI。
- Harmony HTTP/WebView/credential/file/TTS/UI 留在 HarmonyOS App。

退出条件：

- 三端都能创建同一个 Rust runtime。
- 三端都能发送 command、接收 event、处理 host request。
- 三端都能跑最小阅读链路 smoke。

### 阶段 5：跨平台数据、缓存、同步和恢复

目标：Rust Core 统一数据语义，平台只提供目录、权限和系统服务。

范围：

- SQLite schema 和 migration。
- Cache/progress/history/download queue。
- WebDAV sync package、conflict、diff、恢复。
- 账号/凭据不进 Core 明文，凭据 handle 由 host 管理。
- 三端 snapshot/import/export 行为一致。

退出条件：

- 同一数据 fixture 在 CLI/iOS/Android/HarmonyOS 得到一致 hash。
- 冲突、恢复、离线、重复导入有测试。

### 阶段 6：跨平台 benchmark 与发布 gate

目标：证明“三端真的共用同一个 Rust Core 并读出同样结果”。

必须有：

- CLI benchmark runner。
- iOS benchmark runner 或 App-side automation。
- Android benchmark runner 或 instrumentation。
- HarmonyOS benchmark runner 或 device smoke。
- canonical DTO/hash schema。
- release blocker register。

不能接受：

- 只用 Core-side smoke 声明 App/device 完成。
- 只用静态报告声明迁移完成。
- 只用单端结果声明三端一致。

## 并行分支建议

### 分支 A：旧 Reader-Core 代码审计

目标：只读审计 `/Users/minliny/Documents/Reader-Core`，产出迁移任务图。

写入范围：

- `docs/migration/**`
- `reports/migration/**`

提示词：

```text
你在本地 Reader 工作区执行迁移审计。先运行 pwd、find .. -maxdepth 2 -type d -name .git，并对 Reader-Core、Reader for iOS、Reader for Android、Reader for HarmonyOS、Rust 目标仓库分别执行 git status/branch/log。
目标：阅读 /Users/minliny/Documents/Reader-Core 的实际代码，产出迁移到 Rust Reader-Core 的任务图。
本地仓库是唯一事实来源，远程和历史文档只能补充。
不要修改旧 Reader-Core。只在 Rust 目标仓库的 docs/migration 或 reports/migration 下写报告。
每个能力必须映射为：迁移到 Rust、保留为平台 adapter、废弃、延后，并标出代码路径、测试路径、风险、验证命令。
```

### 分支 B：Rust ABI 与协议冻结

目标：稳定 `include/reader_core.h`、`crates/reader-ffi`、`crates/reader-contract`、
`crates/reader-runtime`。

验证：

- `cargo test -p reader-contract -p reader-runtime -p reader-ffi`
- `cargo run -p reader-cli -- --conformance`
- `./scripts/ffi-smoke.sh`

### 分支 C：Rule/JS/Request 迁移

目标：把旧 Core 的规则、JS、网络请求描述迁移到 Rust。

验证：

- `cargo test -p reader-rule -p reader-js -p reader-content`
- 旧 Core fixture replay。

### 分支 D：Local/Storage/RSS/Sync 迁移

目标：把本地书、SQLite、缓存、RSS、WebDAV、同步和恢复迁移到 Rust。

验证：

- `cargo test -p reader-storage -p reader-local-book -p reader-rss -p reader-sync`

### 分支 E：三端 host adapter 接入

目标：iOS/Android/HarmonyOS 通过 Swift/JNI/Node-API 消费同一个 Rust Core。

限制：

- 平台 wrapper 不定义业务语义。
- ABI 变更必须回到 Rust Core 分支。
- App/device proof 必须真实运行，不能用 wrapper compile 替代。

## 当前状态声明

当前仓库已经有 Rust runtime、C ABI、iOS/Android/Harmony wrapper、BookSource raw
兼容、Legado DSL executor、storage/local-book fixture gates、corpus gate 工具等基础
工作。但这些只表示 Rust 目标仓库已有可用基础。是否已经完成迁移，必须重新由本地
`Reader-Core`、`Reader for iOS`、`Reader for Android`、`Reader for HarmonyOS` 的实际
代码和跨平台验证结果判定。

当前仍不能声明完成的关键项：

- request descriptor / cookie / redirect / retry / charset 的 Core-host 契约未完整闭环。
- JS lane 只处理 pure/runtime helper，不处理真实网络、WebView、验证码或文件能力。
- iOS/Android 已有 Native 侧 smoke 证据，但都还不是 host App/device proof。
- HarmonyOS 缺签名 HAP real-device proof。
- corpus 工具已合入，但还没有 CLI + iOS + Android + HarmonyOS 同一 corpus 的零差异结果。
