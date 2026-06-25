# 本地仓库迁移指令

日期：2026-06-25

本文是当前 Reader 迁移工作的最高优先级文档。仓库内其他规划、审计、报告、状态文档
若与本文冲突，以本文为准；历史报告只保留证据价值，不再作为当前实施路线。

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

目标 Rust 仓库默认名为：

- `Reader-Core-Rust`

如果本地目录名称不同，必须先扫描当前工作区定位对应 Git 仓库，不得假设仓库不存在，
也不得优先依赖远程 GitHub。

当前本机扫描结果：

- 未发现 `Reader-Core-Rust`。
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

| 仓库 | 分支 | 状态摘要 | 最新提交摘要 |
| --- | --- | --- | --- |
| `/Users/minliny/Documents/Reader-Core-Native` | `codex/full-branch-directory-consolidation` | clean，与远端同步 | `86ecfc4 docs: localize project documentation to chinese` |
| `/Users/minliny/Documents/Reader-Core` | `main` | dirty，大量删除/修改/新增文件 | `cc7ae849 feat: close core capability gaps` |
| `/Users/minliny/Documents/Reader for iOS` | `main` | dirty，多处删除/修改/新增文件 | `3371d81 chore: sync Reader iOS workspace state` |
| `/Users/minliny/Documents/Reader for Android` | `main` | clean | `ef73081 修复: 移除残留 UI 包引用 (ProjectSkeletonTest + ContractReport)` |
| `/Users/minliny/Documents/Reader for HarmonyOS` | `codex/harmony-napi-runtime` | dirty，多处删除/修改/新增文件 | `d7fe612 feat: wire harmony reader core napi runtime smoke` |

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
