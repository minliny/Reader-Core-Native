# 当前 PR 依赖与证据矩阵

日期：2026-06-26

本文是 `docs/LOCAL_REPO_MIGRATION_DIRECTIVE.md` 和
`docs/MAINLINE_EXECUTION_PLAN.md` 之下的滚动执行快照。它只记录当前分支/PR
依赖、fixture 迁移登记和 proof matrix，不替代主线阶段定义。

## 事实来源

本轮已重新扫描本地与远端状态：

- `Reader-Core-Native` 当前工作树位于
  `/Users/minliny/Documents/Reader-Core-Native`，当前分支是
  `codex/booksource-domain-compat`。
- Native 远端 `origin/main` 为 `0d6f01a`，已经包含 PR #20、#19、#18、#16、
  #15、#14、#13、#12、#2 等已合入工作。
- GitHub 当前打开的 Native PR：
  - PR #21 `codex/corpus-real-run-collector`
  - PR #22 `codex/remote-reading-legado-fixture-corpus`
  - PR #23 `codex/corpus-booksource-oracle-diff` draft
- 本地平台仓库当前状态：
  - iOS：`/Users/minliny/Documents/Reader for iOS`，
    `codex/ios-real-app-core-evidence`，PR #10 draft。
  - Android：`/Users/minliny/Documents/Reader for Android`，
    `codex/android-real-core-runtime-evidence`，PR #2 open。
  - HarmonyOS：`/Users/minliny/Documents/Reader for HarmonyOS`，
    `codex/harmony-signed-device-runtime`，PR #4 open。

## 明确 merge 顺序

| 顺序 | 分支 / PR | 当前状态 | 为什么在这里 | 合入前动作 |
| --- | --- | --- | --- | --- |
| 0 | 主工作树卫生 | 当前 `codex/booksource-domain-compat` 仍有 runtime/protocol/rule 未提交残留 | 防止把不同 lane 的残留随 BookSource PR 带入 | 先按本文“主工作树残留 reconcile”拆分或清理，不从 dirty tree 直接合并 |
| 1 | `codex/runtime-host-capability-contract` | 远端分支存在，领先 `origin/main` 1 个 commit；当前 PR 列表未看到打开 PR | 先固定 Core-owned host capability 枚举、schema、conformance，避免后面 BookSource/Corpus/adapter 重复改协议面 | 新开/恢复 PR；以 contract/runtime/protocol/CLI 为唯一范围 |
| 2 | `codex/booksource-domain-compat` | 远端分支存在，当前工作分支，领先 `origin/main` 1 个 commit；当前 PR 列表未看到打开 PR | BookSource domain normalization 依赖稳定 CLI/protocol 表面；它也改 `tools/reader-cli/src/main.rs`，需在 runtime host contract 后 rebase | rebase 到步骤 1 后，确认只保留 BookSource/domain/content/CLI fixture 相关改动 |
| 3 | PR #22 `codex/remote-reading-legado-fixture-corpus` | open，mergeable | Runtime/Host fixture replay 应在 BookSource 语义稳定后接入，给 corpus oracle 提供可回放输入 | 确认不重新定义 host capability；只作为 fixture/replay corpus |
| 4 | PR #23 `codex/corpus-booksource-oracle-diff` | draft，mergeable | BookSource oracle diff 可以独立推进，但应消费步骤 2/3 的稳定 fixture 与 CLI 输出 | 保持 tools/scripts/tests/samples 范围，不回灌到 BookSource 分支 |
| 5 | `codex/corpus-real-run-lane` | 远端分支存在，领先 `origin/main` 1 个 commit；无对应 open PR | 小范围 real-run collector 只做证据收集，适合接在 oracle 后 | 优先把 PR #21 retarget/split 到这个小分支，避免提前合入 rule/content 大改 |
| 6 | PR #21 `codex/corpus-real-run-collector` | open，mergeable，但分支领先 `origin/main` 2 个 commit 且包含大量 `reader-rule` / `reader-content` 改动 | 该分支已经越过证据收集范围，容易和后续 rule kernel 回归产生大冲突 | 暂不按原样合入；先拆出 collector-only 改动，rule/content 留给 rule-kernel lane |
| 7 | Native iOS / Android adapter evidence | Native PR #12、#2 已合入；老分支落后 `origin/main` | Native binding 证据已在主线，不应阻塞 corpus 工具 | 只在 proof matrix 中引用，不再重复合并旧 Native adapter 分支 |
| 8 | 平台 iOS / Android evidence PR | iOS PR #10 draft；Android PR #2 open | 平台真实 App/runtime evidence 需要等 runtime/BookSource/corpus 输出口稳定后对齐 | 以平台仓库 PR 为准，不把平台能力声明成 Native Core 完成 |
| 9 | HarmonyOS runner/evidence | Native repo 未发现对应 runner 分支；HarmonyOS repo PR #4 open | HarmonyOS runner 归属在 `Reader for HarmonyOS`，不是当前 Native repo | 继续跟踪 HarmonyOS PR #4，不急着在 Native 写 adapter 代码 |

关键结论：`corpus-real-run-collector` 不能作为“纯证据收集”直接排在 oracle 后合入；
当前它夹带 rule/content 大改。为了等规则内核回来时减少冲突，应优先使用或重建
`corpus-real-run-lane` 的 collector-only 版本。

## Legacy Core Fixture Extraction 登记表

| 区域 | 滚动状态 | 当前证据 | 下一步 |
| --- | --- | --- | --- |
| BookSource | covered | PR #4 已合入基础 BookSource 兼容；`codex/booksource-domain-compat` 继续做 domain normalization | 步骤 2 合入后更新 covered 证据到 domain-level |
| Runtime/Host | covered | PR #18/#19 已合入 host replay/request fields；`codex/runtime-host-capability-contract` 补 host capability contract | 步骤 1 合入后把 capability enum/schema/conformance 记入 covered |
| Corpus oracle | covered | PR #13/#20 已合入基础 corpus gate/four-platform fixtures；PR #23 补 BookSource oracle diff | PR #23 合入后把 BookSource five-pipeline oracle 标为 covered-current |
| Rule DSL | pending-rule-kernel | PR #15 只代表最小 DSL executor；当前主工作树有未归属的 `##` replacement 残留 | 等 rule-kernel lane 接管，不混进 runtime/BookSource/corpus collector |
| iOS / Android / HarmonyOS | needs-platform-corpus-proof | Native wrapper/JVM 证据已合入；平台仓库各有 real evidence 分支/PR | 需要统一 corpus output，与 Native corpus collector 对齐后再升级状态 |

## 当前 proof matrix

| 证据线 | 仓库 / 分支 / PR | 已有 proof | 当前状态 | 不能声明 |
| --- | --- | --- | --- | --- |
| BookSource domain | Native `codex/booksource-domain-compat` | content/domain canonical BookSource fixture、CLI fixture vertical 扩展 | remote branch exists；当前主工作树有其他残留，需先拆分 | 不能把 domain normalization 声明为完整 Legado 规则执行 |
| Runtime/Host capability | Native `codex/runtime-host-capability-contract` | `HostCapability` enum、schema extension、host capability conformance fixtures | remote branch exists；应先开/恢复 PR | 不能把 host-owned HTTP/WebView/cookie/file 实现声明成 Core 已完成 |
| Host replay corpus | Native PR #22 `codex/remote-reading-legado-fixture-corpus` | CLI host replay corpus | open / mergeable | 不能替代平台真实 run |
| BookSource oracle diff | Native PR #23 `codex/corpus-booksource-oracle-diff` | canonical BookSource five-pipeline JSON、match/mismatch/missing-platform samples、tooling tests | draft / mergeable | 不能声明四端真实一致，只是 oracle/diff gate |
| Real-run collector | Native `codex/corpus-real-run-lane` preferred; PR #21 currently points to `codex/corpus-real-run-collector` | collector-only branch存在；PR #21 分支包含额外 rule/content 改动 | 需要 split/retarget | 不能让 collector PR 先带入 rule kernel |
| Native iOS adapter | Native PR #12 `codex/ios-rust-host-adapter` | `bindings/ios` shell smoke/status evidence | merged | wrapper smoke 不是 iOS App/device proof |
| Native Android adapter | Native PR #2 `codex/android-host-adapter` | JVM host adapter、schema validation、host runtime tests | merged | JVM adapter 不是 Android `.so`/AAR/device proof |
| iOS app evidence | iOS repo PR #10 `codex/ios-real-app-core-evidence` | native core app evidence gate | draft / mergeable | 不能代表 Android/HarmonyOS 或四端 corpus parity |
| Android runtime evidence | Android repo PR #2 `codex/android-real-core-runtime-evidence` | native runtime evidence bridge | open / mergeable | 不能代表 full app/device reading parity |
| HarmonyOS runner | HarmonyOS repo PR #4 `codex/harmony-signed-device-runtime` | signed real-device evidence runner branch | open / mergeable | 不能在 Native repo 伪造 HarmonyOS adapter completion |

## HarmonyOS 缺口定位

当前 `Reader-Core-Native` 中没有看到对应的 HarmonyOS runner 分支。HarmonyOS 证据线在
`/Users/minliny/Documents/Reader for HarmonyOS` 承担：

- 当前本地分支：`codex/harmony-signed-device-runtime`
- 当前打开 PR：Reader-for-HarmonyOS PR #4
  `feat(harmony): add signed real-device evidence runner`
- 已合入的相关 PR：
  - PR #2 `codex/harmony-napi-runtime`
  - PR #3 `codex/harmony-real-device-evidence`

因此下一步不是在 Native repo 直接写 HarmonyOS adapter 代码，而是把 HarmonyOS PR #4
产出的 evidence artifact 接入 Native corpus collector/proof matrix。

## 主工作树残留 reconcile

当前主工作树在 `codex/booksource-domain-compat` 上仍有未提交改动：

- runtime/protocol/CLI：
  - `crates/reader-contract/src/event.rs`
  - `crates/reader-contract/src/host.rs`
  - `crates/reader-contract/src/lib.rs`
  - `crates/reader-runtime/src/remote.rs`
  - `crates/reader-runtime/src/runtime.rs`
  - `protocol/reader-command.schema.json`
  - `protocol/reader-event.schema.json`
  - `tools/reader-cli/src/conformance.rs`
  - `tools/reader-cli/src/main.rs`
  - `protocol/fixtures/conformance/host/request-unsupported-capability.json`
- rule:
  - `crates/reader-rule/src/lib.rs`
  - `crates/reader-rule/tests/legado_css_dsl.rs`
- untracked worktree dirs:
  - `.wt-corpus-release-gates/`
  - `.wt-data-storage-local-book/`

Reconcile 结论：

- runtime/protocol/CLI 残留与 `origin/codex/runtime-host-capability-contract` 明显重叠，
  但当前残留只是该分支能力的一个较小子集；它不应保留在
  `codex/booksource-domain-compat` 上。处理方式应是切回 runtime host contract PR
  评审完整分支，或在确认远端完整分支覆盖后从 BookSource 工作树移除这些残留。
- rule 残留中的 `##` regex replacement 测试与实现，在当前已推送的
  `corpus-real-run-lane`、`corpus-real-run-collector`、`reader-js-compat-runtime`、
  `legado-rule-dsl-executor`、`runtime-host-capability-contract` 中没有同名覆盖。它应归到
  `pending-rule-kernel`，不要随 runtime/BookSource/corpus collector 合入。
- 两个 `.wt-*` 目录是未跟踪工作树残留，本文不删除。若清理，必须先确认其中没有当前
  agent/用户未同步工作。

清理顺序建议：

1. 保存当前 dirty patch 备份或用专门 stash 标记 runtime/rule 残留。
2. 在 `codex/runtime-host-capability-contract` 上确认完整 runtime/protocol/CLI 证据。
3. 在 `codex/booksource-domain-compat` 上只保留 BookSource domain 相关 diff。
4. 把 `##` replacement patch 转移到 rule-kernel lane，或至少保留为独立 patch 文件等待
   rule kernel 回来后重放。
5. 确认 `.wt-*` 目录 owner 后再清理。
