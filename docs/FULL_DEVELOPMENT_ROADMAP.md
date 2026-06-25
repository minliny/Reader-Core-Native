# Reader-Core-Native 全量开发路线

日期：2026-06-25

基线分支：`codex/full-branch-directory-consolidation`

本文是分支和工作区合并后的全量目标与开发路线。它明确纠正项目方向：
Reader-Core-Native 不是一个泛化的新阅读引擎，而是一个以 Legado 能力兼容为
目标、以旧 Reader-Core 行为资产为迁移依据、以 Native Core/C ABI 为全平台
接入方式、以 corpus benchmark 为最终证明的重建项目。

## 最终目标

构建一个 Rust Native Core。它必须能通过稳定版本化的 C ABI 被 iOS、Android、
HarmonyOS、CLI 以及未来宿主加载，同时兼容本地 Legado 源仓库定义的核心能力，
并迁移或回放旧 Reader-Core 中已经完成的行为、测试、fixture 和证据资产。

项目完成的判定不是“能跑通 smoke”，而是同一个已审批、已脱敏的 corpus case
能在 CLI、iOS、Android、HarmonyOS 包装层中读出一致的 canonical book、TOC、
chapter、progress、content 结果与 hash。

## 事实来源顺序

1. **Legado 定义要兼容什么。**
   `/Users/minliny/Documents/legado` 是只读能力基线。只能做源路径审计和行为
   归纳，禁止复制、翻译或改写 GPL 实现代码。
2. **旧 Reader-Core 定义已有能力怎么迁移。**
   `/Users/minliny/Documents/Reader-Core` 是只读行为资产库。每个有价值的测试、
   fixture、报告、适配器都必须归类为 `migrate`、`replay`、`host` 或 `archive`。
3. **Reader-Core-Native + C ABI 定义全平台怎么接入。**
   Core 负责确定性阅读语义、协议、ABI、错误和数据模型；宿主负责网络、
   WebView、登录、权限、安全存储、UI 和打包。
4. **corpus benchmark 证明兼容声明。**
   没有 CLI 和三端 wrapper 的 canonical 结果证据，就不能声明 Legado parity、
   Core parity 或 production ready。

## 硬性约束

- Legado 只读。
- 不复制、不翻译、不改写 Legado 的 GPL 实现代码。
- Native 开发期间不修改旧 Reader-Core，除非任务明确要求操作旧仓库。
- 平台 App 仓库默认只是集成证据来源，除非任务明确要求操作该平台仓库。
- Core-side smoke 不能冒充 App/device proof。
- 静态报告不能替代运行时证据或 corpus 证据。
- 真实 corpus 必须经过授权、隐私审计和脱敏。
- 没有 corpus 证明的分支可以算基础设施，但不能算 Legado parity closure。

## 当前合并基线

当前 Native 主工作区：

```text
/Users/minliny/Documents/Reader-Core-Native
```

当前分支已经合并这些工作流：

- runtime/protocol 与 conformance fixtures
- 稳定 C ABI 边界、C/C++ smoke
- Android JNI 与 Java/Kotlin wrapper 形态
- Harmony NAPI 与 ArkTS wrapper 形态
- iOS XCFramework/Swift wrapper smoke
- data subsystem：storage、local TXT book、RSS、sync packages/journal
- rule/JS 兼容性补强
- host app contracts、CI gate 设计、release evidence、seed corpus

外部目录仍然是独立输入，不属于 Native 待清理 worktree：

| 路径 | 角色 |
| --- | --- |
| `/Users/minliny/Documents/legado` | 只读兼容能力基线 |
| `/Users/minliny/Documents/Reader-Core` | 旧 Core 迁移和行为证据来源 |
| `/Users/minliny/Documents/Reader for iOS` | iOS 宿主集成目标和证据来源 |
| `/Users/minliny/Documents/Reader for Android` | Android 宿主集成目标和证据来源 |
| `/Users/minliny/Documents/Reader for HarmonyOS` | HarmonyOS 宿主集成目标和证据来源 |
| `/Users/minliny/Documents/Reader UI` | UI/设计来源，不作为 Core 真相 |

## 兼容范围

Legado 能力账本至少要覆盖以下区域：

| 区域 | 需要确定的 Core/host 边界 |
| --- | --- |
| Rule DSL 与链式执行 | Core 负责 selector、fallback、chaining、transform 和错误语义 |
| JSONPath/CSS/XPath/Regex | Core 负责兼容抽取行为和边界用例 |
| JS helper/runtime | Core 负责确定性的 sandbox JS；WebView-only 行为转为 host-required |
| URL/request DSL | Core 负责 request descriptor 和策略；host 执行网络/WebView |
| webBook 链路 | Core 负责 source import、search、detail、TOC、content、pagination、identity、cache、progress |
| Cookie/session/login | Core 负责脱敏 session 语义；host 负责 WebView/平台 store 获取 |
| Local book | Core 负责已支持格式的解析语义，以及未支持格式的明确错误 |
| RSS | Core 负责 source import、parse、refresh、read/starred state；host 负责 fetch |
| Storage/cache/downloads | Core 负责 schema、migration、snapshot、progress、queue 语义 |
| WebDAV/backup/sync | Core 负责 package、journal、conflict 语义；host 负责 WebDAV transport 和凭据 |
| Web API/export/import/admin | 明确 Core contract、product-gated 或 host-owned |

## 旧 Reader-Core 迁移分类

每个旧 Reader-Core 资产必须先归类，才能计入迁移进度：

| 分类 | 含义 | Native 动作 |
| --- | --- | --- |
| `migrate` | 行为或测试应迁移到 Native 实现 | 用 Rust 测试/fixture clean-room 重建 |
| `replay` | 旧输出只作为期望结果证据 | 转为 corpus expected DTO/hash 或 conformance fixture |
| `host` | 平台适配器/UI 行为仍属于宿主 | 定义 host contract 和平台证明要求 |
| `archive` | 不属于 Native parity 目标 | 记录不迁移原因 |

优先级最高的旧 Core 输入：

- parser/rule DSL 测试
- request DSL、URLSession/HTTP adapter、headers/body/charset/retry
- cookie/session/login 脱敏行为
- RECOVERY-29 JS/runtime 行为
- RECOVERY-30 cookie/session/login bridge
- RECOVERY-31 local book ingestion
- RECOVERY-32 local book library/runtime
- RECOVERY-33 unified remote/local reading runtime、cache、progress、downloads
- WebDAV、backup、sync、export/import 报告

## 目标架构

```text
Legado 源码审计
  -> compatibility ledger

旧 Reader-Core 审计
  -> migration/replay/host/archive ledger

Reader-Core-Native
  -> protocol schemas and conformance fixtures
  -> Rust runtime, rule, JS, content, storage, local, RSS, sync crates
  -> C ABI: include/reader_core.h + reader-ffi
  -> platform SDK wrappers: Swift / JNI / NAPI

Host apps
  -> URLSession / OkHttp / Harmony HTTP
  -> WebView login/captcha/cookie capture
  -> file permissions, secure storage, background work, UI, packaging

Corpus benchmark
  -> CLI canonical result
  -> iOS canonical result
  -> Android canonical result
  -> HarmonyOS canonical result
  -> release gate decision
```

ABI 必须保持窄边界：runtime lifecycle、command send、cancel、destroy、
status/error boundary、callback-delivered events。平台 wrapper 消费 ABI，
不定义 Core 语义。

## 全量开发阶段

### Phase 0：合并基线

状态：Native 仓库已完成。

退出证据：

- 已观测 Native worktree/branch 合并到 `codex/full-branch-directory-consolidation`
- 当前分支外没有未合并本地 Native 分支
- `cargo test --workspace` 通过
- `cargo run -p reader-cli -- --conformance` 通过
- `./scripts/ffi-smoke.sh` 通过
- Android Java compile smoke 与 JNI C++ syntax smoke 通过

### Phase 1：Legado 能力账本

目标：先定义 Native 必须兼容什么，再允许后续实现声明 parity。

产出：

- `docs/compat/legado-capability-ledger.md`
- `docs/compat/legado-source-index.json`
- 每个能力的 owner：Core、host、product-gated、policy-no-go、out-of-scope
- 每个能力的证据类型：unit test、conformance fixture、corpus replay、
  host-app proof、manual approval 或 policy closure

退出条件：

- 每个实现分支都能指向一个 capability row，或明确标记为 infrastructure。

### Phase 2：旧 Reader-Core 迁移账本

目标：系统保留旧 Core 已经完成的行为资产，而不是凭记忆重做。

产出：

- `docs/migration/reader-core-migration-ledger.md`
- `docs/migration/reader-core-test-port-plan.md`
- 旧测试、fixture、adapter、report、RECOVERY artifact 清单
- 每个资产归类为 migrate、replay、host 或 archive

退出条件：

- 高风险旧 Core 行为都有 Native 目标或明确 archive 理由。

### Phase 3：Native contract 与 ABI freeze

目标：在平台 wrapper 大规模开发前稳定消费边界。

范围：

- protocol versioning 与 JSON schema
- runtime lifecycle、status、shutdown、cancel
- host operation registry、`host.request`、`host.complete`、`host.error`
- C ABI status code、panic guard、last-error、borrowed callback buffer 规则
- ABI create path 对 runtime config 的接入

退出条件：

- CLI 和 C ABI 能驱动 status、shutdown、cancel、host request/complete/error、
  structured error、runtime config，且没有 wrapper-specific 语义。

### Phase 4：Rule、JS、Request 兼容

目标：关闭 Legado 高风险 rule-chain gap。

范围：

- JSONPath filters、slices、unions、recursive descent、truthiness、missing values
- CSS selector、`:contains`、`:containsOwn`、attribute、text/html extraction
- XPath predicates/namespaces 与 XML feed selectors
- regex extraction、replacement、capture groups、chained transforms
- JS helper、host callback registry、timeout/cancel policy
- request descriptor：method、headers、body、charset、redirect、retry、
  cookie/session policy

退出条件：

- sanitized corpus 能通过 rule/JS 路径跑通 `search -> detail -> toc -> content`，
  且 WebView-only 能力不会被 fake pass。

### Phase 5：远程阅读纵切

目标：复现 Legado webBook 端到端阅读语义。

链路：

1. source import
2. search
3. detail
4. TOC
5. chapter content
6. pagination/windowing policy
7. cache/offline read
8. progress update 与 TOC/content 变化后的 remap
9. duplicate chapter detection 与 canonical URL stability

退出条件：

- 同一 source fixture 在 CLI 和至少一个平台 wrapper 中得到相同 canonical
  DTO/hash；最终 release gate 要求三端全部覆盖。

### Phase 6：数据、本地书、RSS、同步

目标：迁移旧 Reader-Core RECOVERY-31/32/33 和 Legado 的非 webBook 能力。

范围：

- TXT parser 完成度，以及 EPUB/PDF/MOBI/UMD 支持策略
- local book library snapshot、duplicate/change detection、lazy chapter read
- RSS import/parse/update/read/starred state
- storage schema、migration、bookshelf query、progress、cache、download queue
- WebDAV backup/restore、sync package、journal、conflict policy

退出条件：

- snapshot/import/export 行为确定且有测试覆盖
- 未支持格式返回明确 policy error
- local/RSS/sync corpus case 能生成 canonical result

### Phase 7：平台 SDK 与宿主 adapter 证明

目标：每个宿主 App 加载同一个 Native Core commit，并实现 corpus 所需的 host
capability。

平台证明：

- iOS：XCFramework、Swift SDK、URLSession 宿主 adapter、WebView/session adapter、
  App-side runtime lifecycle proof
- Android：JNI `.so`、Java/Kotlin SDK、OkHttp 宿主 adapter、WebView/session
  adapter、App-side runtime lifecycle proof
- HarmonyOS：NAPI `.so`、ArkTS SDK、HTTP/WebView/session adapters、HAP/device 或
  platform-real runner proof

退出条件：

- 每个平台都能运行同一 corpus case 的 benchmark driver，并输出同一 canonical
  result hash。

### Phase 8：Corpus benchmark 与 release gate

目标：用可重复、隐私安全的证据证明 parity。

产出：

- 已审批、已脱敏 corpus 与 source manifest
- CLI、iOS、Android、HarmonyOS corpus runner
- canonical DTO schema 与 hash 规则
- per-capability pass/fail report
- release blocker register
- CI/nightly gate matrix

退出条件：

- `coreParityComplete`、`legadoParityComplete`、`productionReady` 只能在 benchmark
  evidence 和 platform proof 存在后置为 true。

### Phase 9：旧 Core 退役

目标：只有 Native 有证据后，才移除重复 Core 依赖。

范围：

- 宿主 App runtime path 切到 Native Core
- 旧 Reader-Core 资产带 migration mapping 归档
- release 文档声明 Native Core 是生产 Core

退出条件：

- 已覆盖能力的任何平台 release path 都不再依赖旧 Reader-Core。

## 可并行的长期 goal 分支

默认基于 `origin/codex/full-branch-directory-consolidation`。每条分支可以读取
整个 workspace，但写入必须限制在自己的 owned paths。

### 分支 1：Legado 兼容能力账本

分支：`codex/goal-legado-compat-ledger`

写入范围：

- `docs/compat/**`
- `reports/compat/**`

长期目标：

- 产出 source-backed capability ledger，定义 Native 必须兼容什么。

限制：

- 可读取 `/Users/minliny/Documents/legado`，不可编辑。
- 不复制、不翻译、不改写 GPL 实现代码。
- 本分支不实现 Native 代码。

验收：

- 每一行 ledger 都引用本地 Legado 源路径
- 每一行都有 owner 和 evidence type
- 不用静态源码阅读结果声明 Native runtime 完成

提示词：

```text
你在 /Users/minliny/Documents/Reader-Core-Native 工作，分支为 codex/goal-legado-compat-ledger。
目标：为 Reader-Core-Native 建立基于本地 Legado 源码的兼容能力账本。
只读使用 /Users/minliny/Documents/legado。禁止复制、翻译或改写 GPL 实现代码。
创建 docs/compat/legado-capability-ledger.md 和 docs/compat/legado-source-index.json。
每个能力记录：Legado 源路径、用自己的话归纳的行为、owner(Core/host/product-gated/policy-no-go/out-of-scope)、所需证据、优先级、依赖的 Native 模块。
不要修改 crates、bindings、protocol、scripts 或平台 App 仓库。
运行本地可执行的 Markdown/链接一致性检查，并报告剩余未知项。
```

### 分支 2：Reader-Core 迁移账本

分支：`codex/goal-reader-core-migration-ledger`

写入范围：

- `docs/migration/**`
- `reports/migration/**`

长期目标：

- 决定旧 Reader-Core 行为、测试、fixture、证据如何迁移到 Native。

限制：

- 可读取 `/Users/minliny/Documents/Reader-Core`，不可编辑。
- 将旧仓库 dirty state 视为观察快照，不直接视为稳定生产真相。
- 本分支不实现 Native 代码。

验收：

- 每个高风险资产都归类为 migrate、replay、host 或 archive
- 覆盖 RECOVERY-29 到 RECOVERY-33
- test-port plan 标出准确 Native 目标路径

提示词：

```text
你在 /Users/minliny/Documents/Reader-Core-Native 工作，分支为 codex/goal-reader-core-migration-ledger。
目标：盘点旧 Reader-Core 行为资产，并为 Native 重建产出迁移账本。
只读使用 /Users/minliny/Documents/Reader-Core，包括 _archived_planning_2026-06-24。
创建 docs/migration/reader-core-migration-ledger.md 和 docs/migration/reader-core-test-port-plan.md。
将相关 test/report/fixture/adapter 分类为 migrate、replay、host 或 archive。覆盖 parser/rule、request/network、JS/WebView、cookie/session、RECOVERY-31/32 local books、RECOVERY-33 unified runtime、RSS、WebDAV/sync、platform adapters。
不要编辑旧 Reader-Core 仓库，也不要修改 Native 实现文件。
最后列出最高风险的缺失迁移项，以及它们应输入到哪些实现分支。
```

### 分支 3：Rule、JS、Request parity

分支：`codex/goal-rule-js-request-parity`

写入范围：

- `crates/reader-rule/**`
- `crates/reader-js/**`
- `crates/reader-content/**`
- `tests/fixtures/**`
- `fixtures/sanitized-corpus/**`
- `reports/rule-js-request-parity/**`

长期目标：

- 关闭 webBook 链路所需的 rule/JS/request 高风险兼容 gap。

限制：

- 不修改 C ABI 签名，除非另写 ABI proposal。
- 不 fake WebView-only 行为，必须返回 host-required error/contract。
- 优先消费 `docs/compat` 和 `docs/migration`；若尚不存在，以
  `reports/legado-migration-master-audit/README.md` 为 seed。

验收：

- `cargo test -p reader-rule`
- `cargo test -p reader-js`
- `cargo test -p reader-content`
- selector/request descriptor 行为有 corpus case 或 fixture 覆盖

提示词：

```text
你在 /Users/minliny/Documents/Reader-Core-Native 工作，分支为 codex/goal-rule-js-request-parity。
目标：在不改平台 ABI 的前提下补强 Legado-compatible rule、JS、request descriptor 行为。
如果 docs/compat 和 docs/migration 已存在，优先使用；否则从 reports/legado-migration-master-audit/README.md 启动。
聚焦 JSONPath/CSS/XPath/Regex 边界、rule chaining、QuickJS helper、host callback policy，以及 method/headers/body/charset/redirect/retry/cookie request descriptor。
在相关 crate 中写测试；需要跨平台 benchmark 覆盖的行为要加入 sanitized corpus fixture。
除非行为已经表达为 host-required contract，否则不要声明 WebView/login parity。
运行 cargo test -p reader-rule -p reader-js -p reader-content，并记录剩余 gap。
```

### 分支 4：远程阅读 corpus runner

分支：`codex/goal-remote-reading-corpus-runner`

写入范围：

- `tools/reader-cli/**`
- `crates/reader-runtime/**`
- `crates/reader-contract/**`
- `protocol/fixtures/**`
- `fixtures/sanitized-corpus/**`
- `reports/corpus-benchmark/**`

长期目标：

- 把 remote reading vertical 变成可 benchmark 的证明链。

限制：

- 不通过未记录的协议变更修改平台 SDK API。
- 不使用 live private source，不持久化原始 credential/cookie。
- 任何 protocol 变更都必须同步 schema 和 conformance fixture。

验收：

- `cargo run -p reader-cli -- --conformance`
- `cargo test -p reader-runtime -p reader-contract -p reader-cli`
- corpus runner 至少能为 CLI case 输出 canonical DTO/hash

提示词：

```text
你在 /Users/minliny/Documents/Reader-Core-Native 工作，分支为 codex/goal-remote-reading-corpus-runner。
目标：建立 source import -> search -> detail -> toc -> chapter content -> progress 的 corpus benchmark 路径。
扩展 CLI runner，让它读取 sanitized corpus manifest 并输出 canonical DTO/hash。只有确有必要时才更新 protocol/conformance。
报告中不得包含原始 source body、token、cookie 或 credential。初始数据使用 fixtures/sanitized-corpus。
不要编辑平台 App 仓库。
运行 cargo run -p reader-cli -- --conformance 和聚焦 runtime/CLI 测试。报告哪些 case 仍是 CLI-only，以及 iOS/Android/Harmony wrapper 执行还缺什么。
```

### 分支 5：数据、本地书、RSS、同步 parity

分支：`codex/goal-data-local-rss-sync-parity`

写入范围：

- `crates/reader-storage/**`
- `crates/reader-local-book/**`
- `crates/reader-rss/**`
- `crates/reader-sync/**`
- `fixtures/sanitized-corpus/**`
- `reports/data-local-rss-sync/**`

长期目标：

- 迁移旧 Core local/RSS/sync 行为，让三端可消费确定性数据 snapshot。

限制：

- 不在 Core 中实现平台 file picker、secure storage 或 WebDAV network。
- 未支持格式必须明确失败，不能假装 EPUB/PDF/MOBI/UMD 已支持。
- Core snapshot 不保存 transport credential。

验收：

- `cargo test -p reader-storage -p reader-local-book -p reader-rss -p reader-sync`
- snapshot import/export round trip 确定
- local/RSS/sync corpus fixture 有 manifest 和 expected hash

提示词：

```text
你在 /Users/minliny/Documents/Reader-Core-Native 工作，分支为 codex/goal-data-local-rss-sync-parity。
目标：基于旧 Reader-Core 迁移证据和 Legado 能力范围，补齐 data、local-book、RSS、storage、WebDAV/sync、backup/restore parity gap。
聚焦 deterministic snapshot、schema validation、duplicate/change detection、lazy read、RSS refresh state、sync package/journal merge rule、明确 unsupported-format error。
不要实现 host-owned network/file picker/secure-storage UI；需要时建模为 host contract。
运行 cargo test -p reader-storage -p reader-local-book -p reader-rss -p reader-sync，并产出 reports/data-local-rss-sync/status.md。
```

### 分支 6：平台 SDK 与宿主 adapter 证明

分支：`codex/goal-platform-sdk-host-adapters`

写入范围：

- `bindings/ios/**`
- `bindings/android/**`
- `bindings/harmony/**`
- `scripts/build-ios-*`
- `scripts/build-android-*`
- `scripts/build-harmony-*`
- 只有任务明确要求时才写平台 App 仓库

长期目标：

- 证明 iOS、Android、HarmonyOS 能通过稳定 C ABI 消费同一个 Native Core，并
  满足 benchmark case 所需的 host operation。

限制：

- 平台 wrapper 只消费 ABI，不定义 Core 语义。
- 没有 Core ABI 分支和 conformance update，不改 ABI signature。
- Device/App claim 必须有 platform-real proof，不能只靠 wrapper compile。

验收：

- iOS wrapper smoke 与 App-side adapter proof
- Android JNI build/smoke 与 App-side adapter proof
- Harmony NAPI build/smoke 加 HAP/device 或 platform-real proof
- 可用时输出每个 wrapper 的 corpus runner 结果

提示词：

```text
你在 /Users/minliny/Documents/Reader-Core-Native 工作，分支为 codex/goal-platform-sdk-host-adapters。
目标：让 iOS、Android、HarmonyOS 通过稳定 C ABI 消费同一个 Native Core，并产出真实平台 host adapter 证据。
不要在平台 wrapper 中改变 Core 语义。如果必须改 ABI 或 protocol，先写 proposal 并在大范围编辑前停止。
实现或验证 wrapper lifecycle、event delivery、host.complete/host.error，以及 corpus benchmark 所需的 HTTP/WebView/session adapter contract。
Core-side wrapper smoke 不是 App/device proof，证据声明必须精确。
运行当前机器具备 SDK 时可执行的 wrapper build/smoke 命令，并记录缺失 SDK/toolchain blocker。
```

### 分支 7：发布 gate 与证据治理

分支：`codex/goal-release-ci-evidence-governance`

写入范围：

- `docs/ci-gates/**`
- `evidence/release-readiness/**`
- `reports/release-gates/**`
- 只有任务明确包含 CI 实现时才写 `.github/**`

长期目标：

- 把 compatibility、corpus、platform proof 模型转成 fail-closed gate 和 release
  decision evidence。

限制：

- 不用 Core smoke 冒充 host/device proof。
- 依赖不可用 SDK 的 CI job 必须 fail-closed，或只放到正确 runner class。
- corpus evidence 必须保持 privacy-safe。

验收：

- gate matrix 将每个 release claim 映射到 command 或 platform proof artifact
- release blocker register 区分 Core、host、corpus、policy、tooling blocker
- 若修改 CI，必须增量且可回退

提示词：

```text
你在 /Users/minliny/Documents/Reader-Core-Native 工作，分支为 codex/goal-release-ci-evidence-governance。
目标：为 Legado-compatible Native Core 建立 fail-closed release gate 和 evidence governance。
把每个 release claim 映射到所需证据：unit、conformance、corpus CLI、iOS wrapper、Android wrapper、Harmony wrapper、App/device proof、privacy approval 或 policy no-go。
不要把 Core-side smoke 声明成 App/device parity。corpus report 必须隐私安全。
除非明确要求，否则不要编辑 .github workflow；保持 docs/evidence design。
产出 reports/release-gates/status.md，并按需更新 docs/ci-gates。
```

## 并行工作规则

- 遵守 owned write paths 的分支可以并行。
- 如果需要共享 contract 变更，先在自己的 report 目录写 proposal，不直接改其他
  分支的 owned files。
- 实现分支应尽快消费 ledger，但不必等待所有分支结束。
- 已完成分支应合入能验证它的最小 integration lane。
- 每次合并前，分支必须回答：
  1. 关闭了哪一条 Legado capability row？
  2. 迁移、回放、host 化或归档了哪一个旧 Reader-Core asset？
  3. 修改了哪一个 Native protocol/C ABI contract？
  4. 哪些 platform wrapper 必须更新？
  5. 哪个 corpus benchmark case 证明 canonical result 一致？

## 发布 readiness 定义

以下条件全部满足前，项目不能声明 release-ready：

- Legado 能力账本存在，高优先级行有 evidence status。
- 旧 Reader-Core 迁移账本存在，高风险资产已交代。
- Native protocol/C ABI 已版本化、已测试、可被 wrapper 消费。
- Remote reading、local book、RSS、storage、sync 关键路径有确定性测试和 corpus case。
- CLI、iOS、Android、HarmonyOS 能跑同一 approved corpus case。
- 关键路径 canonical result hash 一致。
- host-owned gap 被明确跟踪，且不计入 Core parity。
- 每个真实 corpus case 都有隐私和 source approval 状态。

## 立即下一步

1. 启动 `codex/goal-legado-compat-ledger`。
2. 启动 `codex/goal-reader-core-migration-ledger`。
3. 允许实现分支继续，但必须能说明 ledger seed 和 evidence target。
4. 尽早建设 corpus runner，让功能分支能提交证明，而不是孤立 smoke。
5. 平台 wrapper 工作只作为稳定 ABI 的消费证明，不作为定义 Core 行为的地方。
