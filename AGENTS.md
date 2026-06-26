# AGENTS.md — Reader-Core-Native

本文件适用于整个 `Reader-Core-Native` 仓库目录树。任何在此目录内工作的 agent
必须先读 `docs/PROJECT_CHARTER.md`（项目章程）并遵守其红线与主线不变量。

## 强制阅读

- `docs/PROJECT_CHARTER.md` — 项目最高强制文档（背景、目标、架构不变量、红线）
- `docs/LOCAL_REPO_MIGRATION_DIRECTIVE.md` — 迁移执行指令
- `docs/MAINLINE_EXECUTION_PLAN.md` — 主线阶段（S0–S7）顺序

## 不可偏离的红线

1. **能力底线 = Legado**：验收以"能跑通对应 Legado 能力 / 真实 Legado 书源与 RSS"
   为标尺，不用代码量 / 测试数 / 单端 fixture 自证完成。
2. **Core、平台、UI 三方均开发中**：不得武断声称任何能力已完全建立。
3. **迁移保真 + 补齐**：Rust 迁移 Swift Core 已验证实现；Swift Core 也缺的能力
   对照 Legado 新建，不得跳过。
4. **Core / Host 边界**：Core 不开 socket、不碰 WebView、不存明文凭据。
5. **证据分层**：wrapper smoke ≠ App/device proof，simulator ≠ real device，
   单端结果 ≠ 三端 parity。

## 开工前安全检查

```bash
pwd
git status --short
git branch --show-current
git log -5 --oneline
```

确认本地仓库路径、分支与状态后再修改，不得假设仓库不存在或优先依赖远端 GitHub。
本地仓库是唯一事实来源。

## 工作方式

- 先读本地代码，再写方案；方案必须能落到代码、构建和验证。
- 每轮工作必须能回答章程 §9 的五个问题。
- 面向人的文档使用中文；代码标识、路径、命令、API 名可保留原文。
- 不破坏 dirty 宿主仓库中的用户 / 其他 agent 变更。

## 文档优先级

`docs/PROJECT_CHARTER.md` > `docs/LOCAL_REPO_MIGRATION_DIRECTIVE.md` >
`docs/MAINLINE_EXECUTION_PLAN.md` > 其他 roadmap / 审计 / 报告 > 历史归档文档。
冲突时以上层为准。
