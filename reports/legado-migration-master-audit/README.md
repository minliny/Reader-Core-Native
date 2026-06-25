# 历史审计：Legado 迁移主审计

> 当前最高优先级文档是 `docs/LOCAL_REPO_MIGRATION_DIRECTIVE.md`。本文是历史审计
> 记录，只保留证据价值，不再定义当前项目主路线。

日期：2026-06-25

原审计分支：`codex/legado-migration-master-audit`

原基线：`origin/codex/core-product-integration` at `fb4c3a7`

## 当前解释

本报告产生于早期“Legado 兼容 + Reader-Core 迁移”的规划阶段。当前用户指令已经将
项目主线修正为：

1. 本地仓库是唯一事实来源。
2. 旧 `Reader-Core` 是业务能力迁移源。
3. `Reader for iOS`、`Reader for Android`、`Reader for HarmonyOS` 是真实宿主接入目标。
4. Rust 目标仓库统一使用 `/Users/minliny/Documents/Reader-Core-Native`。
5. 最终目标是三端共用同一个 Rust Reader-Core 作为唯一业务内核。

因此，本报告不再作为当前开发路线、能力优先级或事实来源顺序的依据。

## 可保留的证据价值

本报告仍可作为以下历史输入：

- 曾经扫描过 `/Users/minliny/Documents/legado` 的规则、书源、RSS、本地书、
  WebDAV、HTTP/session 等区域。
- 曾经索引过旧 `Reader-Core` 中的 RECOVERY、LEGADO-COMPAT、capability matrix 等
  历史资产。
- 曾经记录过多个 Native worktree 和分支的合并关系。
- 曾经指出 static report、wrapper smoke、Core-side smoke 不能替代真实运行证据。

这些内容只能辅助当前审计，不能覆盖本地 `Reader-Core`、iOS、Android、HarmonyOS 和
Rust 目标仓库的实际代码。

## 当前应使用的文档

- `docs/LOCAL_REPO_MIGRATION_DIRECTIVE.md`
- `docs/FULL_DEVELOPMENT_ROADMAP.md`
- `ARCHITECTURE.md`
- `FEATURE_MATRIX.md`
- `MIGRATION_MAP.md`

## 历史来源清单

| 来源 | 路径 | 当前用途 |
| --- | --- | --- |
| 旧 Reader-Core | `/Users/minliny/Documents/Reader-Core` | 当前迁移源，必须重新按实际代码审计 |
| iOS 宿主 | `/Users/minliny/Documents/Reader for iOS` | 当前 iOS 接入目标 |
| Android 宿主 | `/Users/minliny/Documents/Reader for Android` | 当前 Android 接入目标 |
| HarmonyOS 宿主 | `/Users/minliny/Documents/Reader for HarmonyOS` | 当前 HarmonyOS 接入目标 |
| Rust 目标仓库 | `/Users/minliny/Documents/Reader-Core-Native` | 当前本机定位的 Rust Core |
| Legado | `/Users/minliny/Documents/legado` | 历史兼容参考，不是当前主事实源 |

## 使用限制

- 不能从本报告直接得出当前迁移完成度。
- 不能用本报告替代本地仓库 `git status`、代码阅读、构建和测试。
- 不能把历史 Legado 能力账本当作当前三端 Rust Core 迁移路线。
- 不能从历史 branch/worktree 状态推断当前目录状态。
- 若本文与当前迁移指令冲突，以 `docs/LOCAL_REPO_MIGRATION_DIRECTIVE.md` 为准。
