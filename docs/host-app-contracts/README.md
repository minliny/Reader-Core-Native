# Host App Contracts

> 沉淀 Rust Core 与 iOS / Android / HarmonyOS host app 之间的责任边界。
> 本目录是 **契约文档**，不是实现状态报告。任何条目都不代表对应能力已经
> 实现完成；每条契约只描述 *应当由谁负责*、*验收证据要求*、*风险* 与
> *后续 owner*。

## 范围与硬约束

- **只允许新增和修改 `docs/host-app-contracts/**`。**
- **禁止修改** `protocol/**`、`bindings/**`、`crates/**`、`scripts/**`、
  `README.md`、`ARCHITECTURE.md`、`MIGRATION_MAP.md`、`FEATURE_MATRIX.md`。
  上述文件是本目录的 *上游事实来源*：本目录只引用、不重写它们。
- 当本目录的契约与上游文档冲突时，以 `ARCHITECTURE.md` 和
  `protocol/compatibility.md` 为准；本目录的职责是 *暴露 gap* 并指明
  *后续 owner*，而不是单方面改写上游契约。
- 每一轮完成一个主题域（见下方路线图），提交一个 commit。不声明实现完成。

## 四类归属定义

每一条能力必须归入且仅归入以下四类之一：

| 类别 | 含义 | 谁执行语义 | 谁提供机制 |
|------|------|-----------|-----------|
| **Core-owned** | 语义与决策完全由 Rust Core 拥有；host 只是无语义的透传通道或根本不参与 | Core | Core |
| **Host-owned** | 完全由 host 平台拥有；Core 不接触、不持久化、不决策 | Host | Host |
| **Shared-contract** | 双方各有责任，通过 JSON host bus 协议耦合：Core 定义契约（params/result/error），host 实现具体 OS 能力 | Core 定义 + Host 执行 | Host |
| **Out-of-scope** | 不在 V1 范围内，显式推迟；任何平台都不得在 V1 自行实现后再要求 Core 兼容 | — | — |

判定规则（按顺序短路）：

1. 若能力只依赖业务语义/规则/数据模型 → **Core-owned**。
2. 若能力必须使用平台独占 OS API（TLS socket、Keychain、系统 TTS、
   WebView、文件选择器、后台任务调度）→ **Host-owned** 或
   **Shared-contract**（取决于 Core 是否需要控制语义）。
3. 若 Core 需要控制语义但必须借 host 的 OS 能力执行 →
   **Shared-contract**（走 `host.request` / `host.complete` 总线）。
4. 若 V1 明确不交付 → **Out-of-scope**，并记录推迟原因与回归触发条件。

## 主题域路线图

每一行对应一个独立 commit / 一轮工作。状态以本目录最新提交为准。

| # | 主题域 | 覆盖范围 | 状态 |
|---|--------|----------|------|
| 01 | network / session | HTTP transport、重定向、Cookie 策略与持久化、响应编码、body 解析归属 | 🟡 契约已立 |
| 02 | local storage / sync | SQLite schema、缓存、阅读进度、下载队列、WebDAV、同步/冲突 | 🟡 契约已立 |
| 03 | login / auth | WebView 登录、验证码、凭据安全存储、登录态注入 Core | ⬜ 待定 |
| 04 | local book / files | TXT/EPUB 解析、文件选择、沙箱授权、Core 数据目录管理 | ⬜ 待定 |
| 05 | tts | TTS 文本切片/播放队列 vs 系统发声 | ⬜ 待定 |
| 06 | ui / background | UI/导航/主题、后台任务、通知、App 生命周期与 runtime 销毁 | ⬜ 待定 |

状态图例：⬜ 待定 · 🟡 契约已立（未实现）· ✅ 契约 + 验收证据齐备（仍不代表实现完成）。

## 每个主题域文档的固定结构

每个 `NN-*.md` 文件遵循以下章节，缺一不可：

1. **Scope** — 本域覆盖与不覆盖的子能力。
2. **Capability inventory** — 表格：子能力 | 归属类别 | 当前事实来源。
3. **Contracts** — 对每条 Shared-contract 给出 params / result / error
   契约草案，并标注与现行 `protocol/compatibility.md` 的 gap。
4. **验收证据要求** — 验收该契约成立所需的 *证据*，
   而非实现本身（例如：conformance fixture、三端 host adapter 冒烟日志、
   `runtime.status` 快照）。不声明实现完成。
5. **Risks** — 跨平台行为漂移、协议破坏、生命周期泄漏等。
6. **Follow-up owners** — 后续落实该契约的责任方（Core runtime /
   protocol schema / iOS adapter / Android adapter / Harmony adapter）。

## 上游事实来源

- `ARCHITECTURE.md` §二 模块归属表、§3.4 Host Capability 走消息。
- `FEATURE_MATRIX.md` 能力归属总表与 V1 边界。
- `protocol/compatibility.md` Host Bus Semantics 与 HTTP Transport
  Capability（现行 `http.execute` 契约的权威定义）。
- `MIGRATION_MAP.md` 各平台迁移状态（用于标注 gap，不作为契约来源）。

---

*基线: `origin/codex/core-product-integration` (fb4c3a7)。本目录不声明任何实现完成。*
