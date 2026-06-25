# 清理清单

日期：2026-06-25

本文是当时的清理审计记录，不是破坏性清理操作记录。制作此清单时没有删除
worktree、目录或分支。后续最终清理结果见
`reports/full-consolidation/2026-06-25.md`。

## 当时结论

当时 workspace 尚未清理完成：

- Legado 迁移审计已集中到 `reports/legado-migration-master-audit`。
- 相关 Documents 目录和 Native worktree 已索引。
- 审计分支已提交为 `e2317d8`。
- 当时尚未删除 Native worktree、Documents 目录或本地分支。
- 未修改任何 dirty 仓库。
- 未将完成分支合入 integration base。

## Documents 目录清单

| 路径 | 当时大小 | Git 状态 | 清理决策 |
| --- | ---: | --- | --- |
| `/Users/minliny/Documents/Reader for MacOS` | 4.0K | non-git | 小 placeholder，仅用户确认后可移除或归档 |
| `/Users/minliny/Documents/Reader for Android_design_docs` | 48K | non-git | 设计文档目录，除非确认废弃否则保留 |
| `/Users/minliny/Documents/Reader for Windows` | 100K | git，有 status entries | 需先检查仓库状态，不从 Native 清理 |
| `/Users/minliny/Documents/Reader-Core-Native-legado-master-audit` | 4.3M | clean | 审计输出 worktree，commit 推送或合并后可移除 |
| `/Users/minliny/Documents/Reader for HarmonyOS` | 44M | dirty | 宿主 App 进行中，不从 Native 清理 |
| `/Users/minliny/Documents/legado` | 101M | clean `master` | 只读兼容基线，保留 |
| `/Users/minliny/Documents/Reader-Core-Native-harmony-napi-integration` | 344M | clean | 合并或归档 Harmony 分支前保留 |
| `/Users/minliny/Documents/Reader-Core-Native-rule-js-compat-clean` | 522M | dirty | 需先 commit/test/audit，不删除 |
| `/Users/minliny/Documents/Reader-Core-Native-data-subsystem-storage` | 670M | clean | 合并或 replay data 分支前保留 |
| `/Users/minliny/Documents/Reader UI` | 677M | clean | 独立 UI 仓库，不属 Native 清理 |
| `/Users/minliny/Documents/Reader-Core-Native-c-abi-worktree` | 946M | clean | 合并或 replay C ABI 分支前保留 |
| `/Users/minliny/Documents/Reader for iOS` | 1.3G | dirty | 宿主 App 进行中，不从 Native 清理 |
| `/Users/minliny/Documents/Reader for Android` | 2.1G | clean | Android 宿主集成目标，保留 |
| `/Users/minliny/Documents/Reader-Core-Native` | 3.9G | clean | 活跃 Native worktree，保留 |
| `/Users/minliny/Documents/Reader-Core` | 13G | dirty | 旧 Core 迁移/证据来源，不从 Native 清理 |

## 当时注册的 Native worktree

| Worktree | 分支 | 状态 | 清理决策 |
| --- | --- | --- | --- |
| `/Users/minliny/Documents/Reader-Core-Native` | `codex/reader-core-runtime-protocol` | clean | 保留 |
| `/private/tmp/ci-gate-design-wt` | `codex/goal-ci-gate-design` | clean | docs 合并后可移除 |
| `/private/tmp/goal-host-app-contracts-wt` | `codex/goal-host-app-contracts` | clean | docs 合并后可移除 |
| `/private/tmp/release-evidence-wt` | `codex/goal-release-evidence` | clean | evidence 合并后可移除 |
| `/Users/minliny/Documents/Reader-Core-Native-c-abi-worktree` | `codex/reader-core-c-abi-stable-boundary` | clean | ABI 集成前保留 |
| `/Users/minliny/Documents/Reader-Core-Native-data-subsystem-storage` | `codex/data-subsystem-storage-cache-coverage` | clean | data 集成前保留 |
| `/Users/minliny/Documents/Reader-Core-Native-harmony-napi-integration` | `codex/harmony-napi-integration` | clean | Harmony 集成前保留 |
| `/Users/minliny/Documents/Reader-Core-Native-legado-master-audit` | `codex/legado-migration-master-audit` | clean | audit 合并/推送后可移除 |
| `/Users/minliny/Documents/Reader-Core-Native-rule-js-compat-clean` | `codex/reader-rule-js-compat-clean` | dirty | 不删除 |
| `.claude/worktrees/android-jni-sdk` | `codex/android-jni-sdk` | clean | Android 集成前保留 |
| `.wt-goal-sanitized-corpus` | `codex/goal-sanitized-corpus` | clean | corpus 合并后可移除 |

## 安全清理顺序

1. push 或 merge audit 分支，再移除 audit worktree。
2. 合并或 replay A 类分支，再移除相关 worktree。
3. 对无 worktree 的未合并分支做 per-branch diff audit。
4. 只用 `git branch -d <branch>` 删除已证明合并或明确废弃的分支。
5. 未确认前避免 `git branch -D`、`rm -rf` 和 host-app cleanup。

## 后续状态

后续全量合并已经完成，相关辅助 Native worktree 已清理。当前权威状态以
`reports/full-consolidation/2026-06-25.md` 为准。
