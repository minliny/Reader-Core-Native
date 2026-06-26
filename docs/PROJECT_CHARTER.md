# Reader 项目章程

日期：2026-06-27

本文是 Reader 项目的最高强制文档。它定义项目背景、开发目标、架构不变量和红线。
仓库内所有其他文档（`LOCAL_REPO_MIGRATION_DIRECTIVE.md`、`MAINLINE_EXECUTION_PLAN.md`、
`ARCHITECTURE.md`、`FEATURE_MATRIX.md`、`MIGRATION_MAP.md` 及任何 roadmap、审计、
报告、状态文档）若与本文冲突，以本文为准。

任何 agent goal、短任务、PR、分支判断和合并决定都必须与本文对齐，不得偏离开发主线。

---

## 1. 项目是什么

Reader 是一款对标 **Legado**（开源 Android 阅读器）能力的个人作品。

核心痛点：Legado 只支持 Android。Reader 的目标是把 Legado 的全部能力带到全平台，
并在此之上进一步优化和拓展。

## 2. 能力底线（不变量）

**Reader 的能力底线 = 兼容 Legado 的全部能力。** 包括但不限于：

- 书源解析（Legado 规则语言：CSS / XPath / JSONPath / Regex / 变量 / JS / 链式组合）
- 主题绘制
- RSS 订阅
- 本地书（TXT / EPUB / PDF / Mobi / Umd）
- WebDAV 同步、备份恢复
- 探索发现
- 替换规则 / 字典规则 / 目录规则等独立实体管理
- TTS（系统 TTS + HttpTTS）

验收标尺统一为"能否跑通对应 Legado 能力 / 真实 Legado 书源与 RSS 规则"。
**禁止用代码量、文件数、测试数、单端 fixture 通过来声明某能力已建立。**

本地 `legado` 仓库（`/Users/minliny/Documents/legado`）是只读的兼容语义基线，
定义"要兼容什么"。每轮工作必须能回答：本轮兼容目标来自 Legado 的哪个代码路径
或数据结构。

## 3. 平台规划

- **第一批次**：Android、iOS、HarmonyOS，兼容折叠屏等移动端设备形态
- **后续批次**：Mac、Windows、Linux
- **前端 UI**：独立仓库 `Reader UI` 开发，与 Core 解耦

## 4. 架构分层（不变量）

```
Legado（只读兼容基线，定义"要兼容什么"）
  ↓
Reader-Core（Swift，开发到一半的旧 Core，迁移事实来源）
  ↓ 定义"已有能力如何迁移"
Reader-Core-Native（Rust，唯一业务内核，当前开发重心）
  ↓ C ABI + JSON protocol
三端平台 adapter（Swift / JNI-Kotlin / Node-API-ArkTS）
  ↓
Reader UI（独立前端仓库） + 各平台 UI / 权限 / WebView / 系统服务 / 打包
```

### Core 与 Host 的责任边界

**Core owns（Core 拥有，不开 socket、不碰 WebView、不存明文凭据）**：
规则引擎、JS 沙箱、请求描述符、书源模型、正文抽取、数据语义、缓存、同步、
本地书、RSS、主题、TTS 文本切片与队列语义。

**Host owns（平台拥有）**：
真实 HTTP / TLS、Cookie jar、WebView 登录、Keychain / Keystore、文件授权、
系统 TTS 发声、UI 组件、权限、后台任务、打包分发。

两端通过 JSON protocol（`host.request` / `host.complete` / `host.error`）通信。

## 5. 为什么会有 Rust Core（历史转折）

原规划用 **Swift** 实现 Core（`Reader-Core` 仓库）。在开发阶段接入平台侧时，
识别到 Swift Core 的 C ABI / FFI / 工具链**无法完美兼容适配 Android 和 HarmonyOS**。
因此启动 `Reader-Core-Native`，用 **Rust 重构 Core** 并迁移原 Swift Core 已实现的能力。
Rust 的 C ABI 能被三端原生消费，从根本上解决平台适配问题。

## 6. 关键事实：各方均处于开发中阶段

**Core、平台、UI 三方都只开发到一半，没有任何一方可以说"能力已完全建立"。**

- 原 Swift Core 虽规模大（19 万行），但本身也未达到 Legado 全部能力
  （TTS / 主题 / 部分 SQLite 持久化 / 部分本地书格式缺失）
- Rust Core 当前对标 Legado 约 30%，真实 Legado 书源端到端跑不通
- 三平台各自处于不同阶段，且能力散落、未以 Core 为唯一业务来源
- Reader UI 是 HTML 原型，非任何平台的可运行 UI

**判断进度时必须保持底线 = Legado，不得武断声称任何项目能力已完全建立。**

## 7. 仓库关系

| 仓库 | 角色 |
| --- | --- |
| `legado` | 只读兼容语义基线 |
| `Reader-Core` | Swift 旧 Core，开发到一半，迁移事实来源（冻结新功能） |
| `Reader-Core-Native` | Rust 唯一业务内核（当前工作重心） |
| `Reader for iOS` | iOS 平台 + adapter |
| `Reader for Android` | Android 平台 + adapter |
| `Reader for HarmonyOS` | HarmonyOS 平台 + adapter |
| `Reader UI` | 独立前端仓库 |
| `Reader for MacOS` / `Reader for Windows` | 后续批次（当前空仓库） |

## 8. 两条红线（贯穿整个项目）

### 红线 1：能力底线 = Legado

每块功能以"对标 Legado 对应能力、能跑通真实 Legado 书源 / RSS 规则"为验收。
不用代码量、测试数、单端 fixture 自证完成。单端通过 ≠ 三端一致，
simulator ≠ real device，工具存在 ≠ benchmark 完成。

### 红线 2：迁移保真 + 补齐到 Legado

Rust 侧迁移原 Swift Core 已验证的实现，不是重新发明；Swift Core 是迁移事实来源。
**但 Swift Core 本身也未完备**，对 Swift Core 也缺的能力（TTS / 主题 /
SQLite 持久化 / TxtTocRule / Bookmark 等），需对照 Legado 源码新建，
不能因 Swift Core 没有就跳过。

## 9. 主线不变量

Reader 迁移只有一条主线：

```
Legado 定义要兼容什么
  -> 旧 Swift Core 定义已有能力如何迁移
  -> Reader-Core-Native 用 Rust Core + C ABI 定义全平台接入边界
  -> 三端平台退役独立业务实现，改为消费同一个 Rust Core
  -> corpus benchmark 证明 CLI / iOS / Android / HarmonyOS 读出同样结果
```

详细阶段顺序见 `docs/MAINLINE_EXECUTION_PLAN.md`（S0–S7）。任何开发分支都必须能回答：

1. 本轮兼容目标来自本地 `legado` 的哪个代码路径或数据结构。
2. 本轮迁移资产来自本地 `Reader-Core` 的哪个代码路径、测试或 sample
   （若 Swift Core 也缺，标注"对照 Legado 新建"）。
3. 本轮 Rust 改动落在哪个 crate、protocol schema、C ABI 或 binding。
4. 本轮是否改变三端 host adapter 的责任边界。
5. 本轮证据是 crate test、CLI conformance、FFI smoke、wrapper smoke、
   App/device proof 还是 corpus benchmark。

不能回答这些问题的工作不能合入主线。

## 10. 防偏离规则

1. **RuleStepSpec 与 Legado DSL 是两套东西**。V1 结构化规则不能冒充 Legado 兼容语言；
   raw Legado DSL 字符串必须保留，只能进入专门的 DSL 执行器。
2. **Core 不开 socket、不直接用 WebView、不保存明文凭据**。
   Core 产出 request descriptor，Host 执行 `http.execute` / WebView / credential / file。
3. **wrapper smoke ≠ App/device proof**。三端阅读链路必须分层标注证据级别。
4. **benchmark 必须四端同 corpus**。差异进入 diff report 和 release blocker register，
   不能用单端结果替代三端 parity。
5. **不破坏 dirty 宿主仓库中的用户 / 其他 agent 变更**。开工前必做安全检查。

## 11. 文档维护要求

- 所有面向人的项目文档使用中文。
- 代码标识、路径、命令、协议字段、API 名称、crate / module / class 名可保留原文。
- 历史审计报告必须标注其历史属性，不能覆盖当前章程与主线。
- 状态快照必须标注扫描日期，不作为永久事实。
- 新增路线、能力矩阵、状态报告必须绑定本地仓库和可执行验证命令。

---

*最后更新：2026-06-27 | 本文为项目最高强制文档，以本文为准则。*
