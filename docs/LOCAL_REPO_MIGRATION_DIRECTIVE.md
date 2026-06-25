# 本地仓库迁移指令

日期：2026-06-25

本文是当前 Reader 迁移工作的最高优先级文档。仓库内其他规划、审计、报告、状态文档
若与本文冲突，以本文为准；历史报告只保留证据价值，不再作为当前实施路线。

详细落地路线见 `docs/MAINLINE_EXECUTION_PLAN.md`。后续所有 agent goal、短任务、
PR 描述和合并判断都必须引用该主线阶段，不得绕开其阶段顺序和防偏离规则。

## 角色与职责

执行者是一名资深跨平台系统架构师与实际开发工程师，目标是把现有 Reader 项目迁移为
以 Rust 为唯一业务内核的三端统一架构。

职责不是只给建议，而是必须实际推进：

- 阅读本地仓库的实际代码。
- 基于代码建立迁移方案。
- 修改代码。
- 构建和验证。
- 按阶段交付可运行结果。
- 最终完成 iOS、Android、HarmonyOS 三个平台共用同一个 Rust Reader-Core。

## 必备技术范围

- Rust
- Swift / SwiftUI
- Kotlin / Android / JNI
- ArkTS / HarmonyOS NEXT / Node-API
- C / C++
- C ABI
- SQLite
- HTTP、Cookie、Session、Redirect
- QuickJS 或其他可嵌入 JavaScript runtime
- CSS / XPath / JSONPath / Regex
- ebook / TXT / EPUB
- WebDAV / RSS / TTS
- 跨平台缓存、同步、恢复和 diff
- 大型代码迁移与 strangler migration

## 本地仓库是唯一事实来源

所有仓库均位于当前本地工作区。优先查找：

- `Reader-Core`
- `Reader-for-iOS`
- `Reader-for-Android`
- `Reader-for-HarmonyOS`

目标 Rust 仓库统一使用：

- `Reader-Core-Native`

后续仍必须先扫描当前工作区定位对应 Git 仓库和分支状态，不得假设仓库不存在，
也不得优先依赖远程 GitHub。

当前本机扫描结果：

- 当前可定位的 Rust 目标仓库为 `/Users/minliny/Documents/Reader-Core-Native`。
- 现有宿主仓库分别为：
  - `/Users/minliny/Documents/Reader-Core`
  - `/Users/minliny/Documents/Reader for iOS`
  - `/Users/minliny/Documents/Reader for Android`
  - `/Users/minliny/Documents/Reader for HarmonyOS`

远程 README、历史讨论、先前架构描述只能作为补充，不能覆盖实际代码。

## 开工前安全检查

每次开始工作前，必须分别检查：

```bash
pwd
find .. -maxdepth 2 -type d -name .git
git -C <repo> status --short
git -C <repo> branch --show-current
git -C <repo> log -5 --oneline
```

当前轮次已执行的检查摘要：

最近一次本地扫描：2026-06-25。后续任何 agent 仍必须重新执行开工前安全检查，本表
只记录本次 checkpoint 的事实快照。

| 仓库 | 分支 | 状态摘要 | 最新提交摘要 |
| --- | --- | --- | --- |
| `/Users/minliny/Documents/Reader-Core-Native` | `main` / `fc5fb57` | 当前 Rust 主线；PR #15/#14/#13/#2/#12 已合并，JS lane 为 PR #16 | `fc5fb57 Merge pull request #12 from minliny/codex/ios-rust-host-adapter` |
| `/Users/minliny/Documents/Reader-Core` | `main` | clean，旧核心迁移源 | `a6db53e0 docs: add Reader-Core to Rust migration ledger` |
| `/Users/minliny/Documents/legado` | `master` | clean，只读 Legado 兼容语义基线 | `da17bb2be 优化 #5784` |
| `/Users/minliny/Documents/Reader for iOS` | `codex/ios-rust-host-adapter` | iOS 宿主迁移目标；Native `bindings/ios` shell smoke 证据已由 PR #12 进入主线 | `7dabaae Update STATUS.md with Round 6 commit hash` |
| `/Users/minliny/Documents/Reader for Android` | `main` | clean | `ef73081 修复: 移除残留 UI 包引用 (ProjectSkeletonTest + ContractReport)` |
| `/Users/minliny/Documents/Reader for HarmonyOS` | `codex/harmony-napi-runtime` | clean，HarmonyOS 宿主迁移目标；PR #2 draft，已有 headless/simulator/package 证据，无 real-device proof | `b504686 docs: refresh NAPI smoke report to three-tier state` |

## 当前工程目标

最终目标是一个 Rust Reader-Core 作为唯一业务内核：

```text
Reader-Core / 旧核心能力
  -> 迁移、回放、验证

Rust Reader-Core
  -> 规则、JS、网络请求描述、书源、阅读、书库、缓存、同步、RSS、本地书、TTS 契约
  -> C ABI

iOS / Android / HarmonyOS
  -> 通过 Swift/JNI/Node-API 消费同一个 Rust Core
  -> 只保留平台能力、UI、权限、WebView、系统服务和打包分发
```

Rust Core 必须成为业务能力的唯一来源。三端不得各自保留独立业务实现并长期分叉。

主线顺序固定为：

```text
Legado 定义要兼容什么
  -> 旧 Reader-Core 定义已有能力如何迁移
  -> Reader-Core-Native 用 Rust Core + C ABI 定义全平台接入边界
  -> corpus benchmark 证明 CLI / iOS / Android / HarmonyOS 读出同样结果
```

因此，BookSource / 规则链路必须先以本地 `legado` 和旧 `Reader-Core` 建立兼容模型、
fixture 和 raw rule 保真，再进入 Rust 执行器、三端 adapter 和 benchmark。不能先按
当前 Rust V1 `RuleStepSpec` 继续扩展后再倒推兼容 Legado。

## 工作方式

- 先读本地代码，再写方案。
- 方案必须能落到代码、构建和验证。
- 每个阶段都要有可运行结果。
- 不用远端文档替代本地仓库审计。
- 不因历史文档存在就跳过当前代码检查。
- 不把 wrapper smoke、静态报告、单端验证声明成三端完成。
- 不破坏 dirty 宿主仓库中的用户/其他 agent 变更。

## 文档维护要求

- 所有面向人的项目文档使用中文。
- 代码标识、路径、命令、协议字段、API 名称、crate/module/class 名可保留原文。
- 历史审计报告必须标注其历史属性，不能覆盖当前迁移指令。
- 新增路线、能力矩阵、状态报告必须绑定本地仓库和可执行验证命令。
