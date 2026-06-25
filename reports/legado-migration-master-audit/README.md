# Legado 迁移主审计

日期：2026-06-25

原审计分支：`codex/legado-migration-master-audit`

原基线：`origin/codex/core-product-integration` at `fb4c3a7`

本审计修正项目方向：Reader-Core-Native 不是泛化的新阅读引擎，而是
Legado 能力兼容 + 旧 Reader-Core 迁移 + Native/C ABI 全平台接入 + corpus
benchmark 证明的重建项目。

## 核心决策

项目目标顺序：

1. 用本地 Legado 源码定义必须兼容什么。
2. 用已有 Reader-Core 定义哪些能力可以迁移、回放或作为行为证据。
3. 用 Reader-Core-Native 和版本化 C ABI 解决真实跨平台消费。
4. 用已脱敏、已审批的 corpus benchmark 证明同一 source、request chain、
   chapter read flow 在三端得到同一 canonical result。

任何从“做一个抽象跨平台阅读 Core”出发的计划都是错误方向。任何把旧 Reader-Core
当作可丢弃历史的计划也错误。旧 Core 是行为和证据资产；Native Core 是跨平台执行
载体。

## 来源清单

| Source | 路径 | 当时状态 | 用途 |
| --- | --- | --- | --- |
| Legado baseline | `/Users/minliny/Documents/legado` | clean `master`，head `da17bb2be` | 只读能力基线；禁止复制、翻译或改写 GPL 实现 |
| Existing Reader-Core | `/Users/minliny/Documents/Reader-Core` | dirty `main`，head `cc7ae849` | 迁移/证据来源；dirty state 需逐项核实 |
| Native 主 worktree | `/Users/minliny/Documents/Reader-Core-Native` | 审计时 clean | 当前 Rust/C ABI 仓库 |
| C ABI worktree | `/Users/minliny/Documents/Reader-Core-Native-c-abi-worktree` | clean | ABI boundary 工作 |
| Data subsystem worktree | `/Users/minliny/Documents/Reader-Core-Native-data-subsystem-storage` | clean | content/local/RSS/storage/sync 工作 |
| Harmony NAPI worktree | `/Users/minliny/Documents/Reader-Core-Native-harmony-napi-integration` | clean | Harmony binding/NAPI smoke 工作 |
| Rule/JS worktree | `/Users/minliny/Documents/Reader-Core-Native-rule-js-compat-clean` | 当时 dirty | rule 与 JS compatibility lane |
| Android JNI worktree | `.claude/worktrees/android-jni-sdk` | clean | Android JNI first slice |
| 脱敏 corpus worktree | `.wt-goal-sanitized-corpus` | clean | seed corpus 脚手架 |
| Host apps | `Reader for iOS`、`Reader for Android`、`Reader for HarmonyOS` | 状态各异 | 平台集成目标和证据来源 |

后续这些 Native worktree 已合并清理，最终状态见
`reports/full-consolidation/2026-06-25.md`。

## Legado 定义要兼容什么

审计到的 Legado 只读源码区域：

| 能力区域 | Legado 源路径 |
| --- | --- |
| Rule parsing 与 rule data | `AnalyzeRule.kt`、`RuleAnalyzer.kt`、`RuleData.kt`、`RuleDataInterface.kt` |
| JSONPath / CSS / XPath / Regex | `AnalyzeByJSonPath.kt`、`AnalyzeByJSoup.kt`、`AnalyzeByXPath.kt`、`AnalyzeByRegex.kt` |
| URL/request DSL | `AnalyzeUrl.kt`、`CustomUrl.kt`、`help/http/*` |
| Web book vertical | `model/webBook/{SearchModel,BookList,BookInfo,BookChapterList,BookContent,WebBook}.kt` |
| 书源模型 | `data/entities/BookSource.kt`、`BookSourcePart.kt` |
| RSS | `model/rss/*`、`RssSource.kt` |
| 本地书格式 | `model/localBook/*`、`modules/book/**` |
| HTTP/Cookie/session | `CookieManager`、`CookieStore`、`HttpHelper`、`OkHttpUtils`、`Cronet`、`BackstageWebView` |
| WebDAV/sync | `lib/webdav/*` |
| Data model / persistence | `data/dao/*`、`data/entities/*` |
| Web API / source web UI | `api.md`、`app/api/**`、`modules/web/src/**` |

兼容账本至少必须覆盖：

1. Rule DSL 语法和链式执行语义。
2. JS/Rhino-like helper 和 host callback 行为。
3. Request DSL：method、headers、body、charset、redirect、retry、error policy。
4. webBook 链路：search -> detail -> toc -> content -> pagination。
5. chapter identity、ordering、duplicate detection、canonical URL stability。
6. cookie/session/login/WebView-hosted flow。
7. RSS source import、parse、update、reading flow。
8. local book import、format detection、chapter/resource read、lazy reading。
9. WebDAV backup/sync 与 conflict behavior。
10. data schema、migration、cache/progress/bookmark/download queue。
11. Web API/export/import/admin surface。

## 旧 Reader-Core 定义迁移资产

旧 Reader-Core 不是废弃代码。它已经有大量行为资产，应迁移或回放。

重要 archive 事实：

- `LEGADO_FULL_CAPABILITY_MATRIX_V2_SOURCE_BACKED_SUMMARY.md`
  - 51 个 capability entry。
  - 25 个 Core-denominator capability。
  - 11 个 host-app scope capability。
  - 11 个 product-gated capability。
  - `productionReady=false`、`legadoParityComplete=false`、
    `coreParityComplete=false`。
  - 当时没有真实 corpus benchmark。
- `LEGADO-COMPAT-1_CAPABILITY_GAP_MATRIX_SUMMARY.md`
  - 82 个 capability entry。
  - 8 supported、35 partial、3 missing、10 product-approval、11 policy-no-go、
    11 not measured。
  - 62 个 high-risk parity gap。
  - 最高风险是 rule-chain 和 corpus parity：search->detail、detail->toc、
    toc->content、chapter identity/order、duplicate chapter detection、
    canonical URL stability。
- `LEGADO-COMPAT-11_EXTERNAL_APPROVAL_ATTESTATION_RESPONSE_GAP_AUDIT_SUMMARY.md`
  - 258 个 follow-up response decision 缺失。
  - approval captured count 仍为 0。
  - benchmark-ready corpus count 仍为 0。

关键 RECOVERY 输入：

| RECOVERY | 旧 Reader-Core 已证明什么 | Native 迁移含义 |
| --- | --- | --- |
| RECOVERY-29 | JS executor、WebView DOM executor、runtime binding result | JS/runtime 行为需谨慎迁移；WebView 仍由 host adapter 负责 |
| RECOVERY-30 | cookie jar、session、login bridge | Core 应拥有 cookie/session 语义；host 提供 WebView/cookie 获取 |
| RECOVERY-31 | TXT/EPUB/PDF format、encoding、chapter/resource | local book 行为和测试不能重做时丢失 |
| RECOVERY-32 | catalog、duplicate/change、lazy chapter/resource、progress/cache | storage/content 工作应保留这些语义 |
| RECOVERY-33 | unified remote/local runtime、offline cache、TOC refresh、downloads、progress remap | Native runtime/storage 应以 unified behavior 为目标 |

旧 Core 测试资产包括：

- `Tests/ReaderCoreParserTests/*`
- `Tests/ReaderCoreNetworkTests/*`
- `Tests/ReaderPlatformAdaptersTests/*`
- `Tests/ReaderCoreModelsTests/*`
- `Tests/ReaderCoreJSRendererTests/*`
- `samples/reports/latest/**`
- `Adapters/Apple/**`
- `Adapters/HTTP/**`

这些是迁移输入，不是 Native 的自动生产证据。

## Native Core / C ABI 解决平台消费

旧 Core 没有成为三端统一加载的引擎。Native Core 必须解决这个问题，但必须在兼容
目标清楚后进行。

边界：

| 层 | Native Core 负责 | Host 负责 |
| --- | --- | --- |
| Protocol | command/event schema、request correlation、conformance | 发送 command、消费 event |
| Runtime | lifecycle、status、shutdown、cancel、pending host operation registry | App lifecycle scheduling、threading integration |
| ABI | C ABI、status/error、panic guard、last_error、event payload | Swift/JNI/NAPI wrapper |
| Rule/request | rule execution、request descriptor、redirect/cookie/encoding policy | TLS/socket/WebView/file permission |
| Data | book/source/progress/cache/download/sync model | OS sandbox directory、secure storage handle、background scheduling |
| WebView/login/captcha | host request contract 与 redacted session import | UI、WebView DOM、captcha/manual approval |

## Corpus benchmark 证明三端结果一致

当前 seed corpus 有 5 个 synthetic fixture：

- `bs-001` book-source JSON
- `wp-001` static HTML page
- `ja-001` JSON API response
- `xf-001` XML/OPDS-style feed
- `rf-001` RSS feed

这只是起点，不是 parity 证明。完整 benchmark 需要：

1. Legado-defined capability corpus。
2. Reader-Core 期望行为 replay corpus。
3. Native CLI canonical result corpus。
4. iOS/Android/Harmony wrapper 执行 corpus。
5. Cross-platform canonical DTO comparison。

最小结果 schema：

```json
{
  "caseId": "source-chain-001",
  "sourceType": "book-source",
  "capabilities": ["search", "detail", "toc", "content", "chapter-identity"],
  "expected": {
    "bookId": "...",
    "title": "...",
    "tocCount": 10,
    "chapterOrderHash": "...",
    "contentHash": "..."
  },
  "runs": {
    "cli": {"status": "pass", "hash": "..."},
    "ios": {"status": "pass", "hash": "..."},
    "android": {"status": "pass", "hash": "..."},
    "harmony": {"status": "pass", "hash": "..."}
  },
  "privacy": {
    "rawBodyPersisted": false,
    "tokensPersisted": false,
    "cookiesPersistedInReport": false
  }
}
```

关键路径没有这类证据时，不能声明 Legado parity。

## 正确全量路线

完整路线已经整理到 `docs/FULL_DEVELOPMENT_ROADMAP.md`。核心阶段：

1. 合并基线与分支卫生。
2. Legado 能力账本。
3. Reader-Core 迁移账本。
4. Protocol 与 C ABI foundation。
5. Rule / JS / request compatibility。
6. 远程阅读链路。
7. Local book / RSS / WebDAV / storage。
8. 真实平台消费。
9. Corpus benchmark 与 release gate。

## 必须新增的长期 goal

- `codex/goal-legado-compat-ledger`
- `codex/goal-reader-core-migration-ledger`
- `codex/goal-rule-js-request-parity`
- `codex/goal-remote-reading-corpus-runner`
- `codex/goal-data-local-rss-sync-parity`
- `codex/goal-platform-sdk-host-adapters`
- `codex/goal-release-ci-evidence-governance`

具体写入边界、限制和提示词见 `docs/FULL_DEVELOPMENT_ROADMAP.md`。

## 不可妥协的守则

- Legado 只读。
- 不复制、翻译或改写 GPL 实现代码。
- Native 审计 agent 不修改 dirty 旧 Reader-Core。
- 宿主 App 仓库默认只是证据来源。
- Core-side smoke 不得报告为 App/device parity。
- 静态报告不能替代 runtime 或 corpus proof。
- 真实 corpus 需要 approval、privacy review 和 redaction。

## 当前结论

Native repo 已经在 runtime/protocol、C ABI、storage/content/sync、Harmony NAPI、
Android JNI、CI design、host contracts、release evidence、seed corpus 上有实质进展。
但项目还必须用单一 Legado parity 账本和旧 Reader-Core 迁移账本重新锚定
所有实现。后续分支不能只说“增加能力”，必须说明它关闭了哪条兼容行、迁移了哪个旧
Core 资产，并通过哪一个 corpus case 证明。
