# 分支集成账本

> 当前最高优先级文档是 `docs/LOCAL_REPO_MIGRATION_DIRECTIVE.md`。本文是历史分支集成
> 记录，不覆盖当前本地仓库迁移路线。

日期：2026-06-25

本文把当时已完成的分支清单转成集成策略。原始审计时并未直接合并全部分支，因为
多条分支带有早期 agent 的重叠历史。正确做法是 path-level integration 或 selective
replay，而不是盲目 merge。

后续完整合并结果见 `reports/full-consolidation/2026-06-25.md`。

## 分支类别

### A 类：独立、低风险整合

这些分支是文档或 corpus-data lane，选定目标基线后可独立合并或 replay：

| 分支 | 原 worktree | 范围 | 验证 |
| --- | --- | --- | --- |
| `codex/goal-ci-gate-design` | `/private/tmp/ci-gate-design-wt` | `docs/ci-gates/**` | Markdown review |
| `codex/goal-host-app-contracts` | `/private/tmp/goal-host-app-contracts-wt` | `docs/host-app-contracts/**` | 与 C ABI event 保持一致 |
| `codex/goal-release-evidence` | `/private/tmp/release-evidence-wt` | `evidence/release-readiness/**` | 不声明 App/device parity |
| `codex/goal-sanitized-corpus` | `.wt-goal-sanitized-corpus` | `fixtures/sanitized-corpus/**`、`reports/corpus-audit/**` | privacy grep、manifest 检查 |

### B 类：Core foundation，需要 path-level audit

这些分支价值高，但与早期 agent history 重叠：

| 分支 | 原 worktree | 范围 | 验证 |
| --- | --- | --- | --- |
| `codex/reader-core-runtime-protocol` | 主 Native worktree | runtime status/shutdown、cancel、host completion、protocol conformance | `cargo test -p reader-contract -p reader-runtime` |
| `codex/reader-core-c-abi-stable-boundary` | `Reader-Core-Native-c-abi-worktree` | `include/reader_core.h`、`crates/reader-ffi`、iOS module map、FFI smoke | `cargo test -p reader-ffi`、`./scripts/ffi-smoke.sh` |
| `codex/data-subsystem-storage-cache-coverage` | `Reader-Core-Native-data-subsystem-storage` | content/local-book/RSS/storage/sync | 对应 data crate tests |

### C 类：平台 lane，等 Core/ABI shape 稳定后集成

平台分支消费 ABI，不设置 Core 语义：

| 分支 | 原 worktree | 范围 | 验证 |
| --- | --- | --- | --- |
| `codex/android-jni-sdk` | `.claude/worktrees/android-jni-sdk` | JNI bridge、CMake、Kotlin sample、command/event bridge | NDK build、JNI smoke、Android App adapter smoke |
| `codex/harmony-napi-integration` | `Reader-Core-Native-harmony-napi-integration` | NAPI wrapper、ArkTS SDK helper、Harmony smoke 产物 | OHOS build、NAPI smoke、HAP/device proof |

### D 类：当时仍活跃，暂不合并

| 分支 | 原 worktree | 范围 | 当时 blocker |
| --- | --- | --- | --- |
| `codex/reader-rule-js-compat-clean` | `Reader-Core-Native-rule-js-compat-clean` | rule 和 JS compatibility | dirty 文件需先提交并跑 focused tests |

该分支随后已提交为 `feat(rule): complete js compatibility parity cases`，并已合并到
全量集成分支。

## 原建议集成顺序

1. 先合并 A 类，形成 source truth、host contract、CI、release、corpus scaffold。
2. 集成 `reader-core-runtime-protocol` 的 core-only 工作。
3. 集成 `reader-core-c-abi-stable-boundary`。
4. 在 frozen ABI 上集成 Android 和 Harmony 平台 lane。
5. 解决 protocol/ABI 冲突后集成 data subsystem。
6. 完成并验证 rule/JS 分支后，将其作为 Legado 兼容执行 lane 集成。

## 合并审查问题

每次集成都必须回答：

1. 关闭了哪条 Legado 兼容能力？
2. 迁移、回放、host 化或归档了哪个旧 Reader-Core asset？
3. 修改了哪个 Native/C ABI contract？
4. 哪些 platform wrapper 必须更新？
5. 哪个 corpus benchmark case 证明 canonical result 一致？

如果第 5 项答案是“没有”，该分支可以作为 infrastructure 合并，但不能计入 Legado
parity closure。
