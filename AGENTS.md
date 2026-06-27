# AGENTS.md — Reader-Core-Native

本文件适用于整个 `Reader-Core-Native` 仓库目录树。任何在此目录内工作的 agent
必须先读 `docs/PROJECT_CHARTER.md`（项目章程）并遵守其红线与主线不变量。

## 强制阅读

- `docs/PROJECT_CHARTER.md` — 项目最高强制文档（背景、目标、架构不变量、红线）
- `docs/LOCAL_REPO_MIGRATION_DIRECTIVE.md` — 迁移执行指令
- `docs/MAINLINE_EXECUTION_PLAN.md` — 主线阶段（S0–S7）顺序
- `docs/LEGADO_CAPABILITY_INVENTORY.md` — Legado 全部能力清单（97 项）与 Reader 对标状态，
  是"能力底线 = Legado"的验收基准。任何"能力已完成"的判断必须对照本清单逐项验证。
- `docs/CAPABILITY_GAP_PLAN.md` — 能力缺口补齐方案（45 项未实现 + 16 项待验证），
  按优先级排序。补齐后必须更新清单状态并用测试工具链验证。
- `docs/STATUS_SNAPSHOT_2026-28.md` — 2026-06-28 项目状态快照（7个能力缺口 agent 完成 + 全量459源批量测试）（仓库状态 + S 阶段
  修正进度 + release blockers + 能力对标 + 测试工具链 + 平台侧状态）。时间点快照，
  不作为永久事实。

## 不可偏离的红线

1. **能力底线 = Legado**：验收以"能跑通对应 Legado 能力 / 真实 Legado 书源与 RSS"
   为标尺，不用代码量 / 测试数 / 单端 fixture 自证完成。
   **必须对照 `docs/LEGADO_CAPABILITY_INVENTORY.md` 逐项验证**，不得凭 agent
   自报或零散测试声称完成。97 项能力中 45 项未实现、16 项部分实现从未用真实
   Legado 数据验证。
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
`docs/MAINLINE_EXECUTION_PLAN.md` > `docs/LEGADO_CAPABILITY_INVENTORY.md` >
`docs/CAPABILITY_GAP_PLAN.md` > `docs/TEST_AND_VERIFICATION_PLAN.md` >
状态快照 `docs/STATUS_SNAPSHOT_*.md`（最新为准）> 其他 roadmap / 审计 / 报告 > 历史归档文档。
冲突时以上层为准。

## TTS 策略（强制）

TTS 是 Reader 对标 Legado 朗读能力的关键模块，以下约束不可偏离：

1. **Core 只做编排，不做合成**：Core（Rust）负责文本切片、播放队列状态机、
   章节边界过渡与朗读位置持久化，**不嵌入任何语音模型、不做语音合成**。
2. **发声归 Host 系统级 TTS**：各平台使用系统原生 TTS（iOS `AVSpeechSynthesizer`、
   Android `TextToSpeech`、HarmonyOS `SystemTts`），与 Legado 架构一致。
3. **HttpTTS 为下一阶段能力**：兼容 Legado HttpTTS 配置格式（百度 / 阿里云 /
   自建服务等），Core 产出请求 descriptor（URL、headers、body、拼接规则），
   Host 执行 HTTP 拉取音频流并喂给播放器；Core 不开 socket。
4. **本地神经声学 TTS 暂不定型**：Sherpa-ONNX 等本地小模型方案等 HttpTTS 验证
   后再评估选型；当前保留"系统 TTS + HttpTTS"双轨策略。
5. **Core 永不嵌入语音模型**：神经声学模型体积 50–200MB，会让三端包体爆炸且
   违反 Core/Host 边界；如需本地神经 TTS，模型由 Host 按需下载加载。

## 书源测试工具链（强制）

书源测试工具链是验证"能力底线 = Legado"的核心基础设施。以下约束不可偏离：

### 工具链架构

```
corpus-manager (Python)          — 书源管理：导入/分类/脱敏/验证
  ↓
reader-cli --test-source (Rust)  — 单源 L1-L5 live 测试（CLI 充当 Host 发 HTTP）
reader-cli --test-corpus (Rust)  — 批量 L1-L5 live 测试
reader-cli --test-corpus-offline — 批量 L1-L5 离线回放（用录像数据，CI PR gate）
  ↓
reports/tooling/                 — 结果产出 + 证据索引 + blocker 自动生成
```

### 5 级通过定义（评估规则）

每个真实 Legado 书源"跑通" = 以下 5 级全部 pass：

| 级别 | 步骤 | 通过条件 |
|------|------|----------|
| L1-import | `source.import` | 返回 result，sourceId 非空，无 error |
| L2-search | `book.search` | ≥1 本书，title 非空 |
| L3-detail | `book.detail` | name 非空 |
| L4-toc | `book.toc` | ≥1 章节，title 非空 |
| L5-content | `chapter.content` | content >50 字符 |

前级 fail 则后续级 skip。一个源"完全通过" = L1-L5 全绿。

### 链式提取（核心机制）

测试工具必须链式提取 URL，不能一次性喂 mock 数据：
- search 结果 → 提取 `bookUrl` → 喂给 detail
- detail 结果 → 提取 `tocUrl` → 喂给 toc
- toc 结果 → 提取 `chapterUrl` → 喂给 content

Core 已就绪（`book.search` 传 `keyword` 自动构造 URL → `http.execute` → Host 执行 →
`complete_remote_host` continuation）。CLI 缺失 HTTP 客户端 + 链式提取，
这是工具链能否工作的关键断链。

### Core/Host 边界在测试工具中的体现

- **CLI 充当 Host**：CLI 用 `ureq` 发真实 HTTP 请求（Core 不开 socket，红线 4 遵守）
- **Core 产出 request descriptor**：Core 发 `http.execute` host request，CLI 执行
- **离线回放不触网**：`--test-corpus-offline` 把录像的 response body 喂给 Core 的
  `xxxResponse` 字段，Core 不发 `http.execute`

### 录像（副产品，非前置步骤）

- live 测试时自动保存每步 HTTP 响应到 `tests/fixtures/corpus/recorded/`
- 录像是 live 测试的副产品，不是先录制再测试
- CI PR gate 用离线回放（稳定），nightly 用 live（真实网络）

### Gate 规则

| Gate | 触发 | 内容 | 通过条件 |
|------|------|------|----------|
| PR | 每次 PR | fmt + clippy + test + conformance + P0 offline 10 源 | 0 fail |
| main | 合并 main | PR + fixture_vertical + P0 offline 30 源 + 证据索引 | P0 ≥80% |
| nightly | 每天 02:00 | main + 全量 459 offline + live smoke 10 源 | 全量 ≥60% |
| release | 手动触发 | nightly + 四端 corpus benchmark | parity ≥90% |

### 跨会话审计要求

任何会话的 agent 在完成书源测试工具链相关工作后，必须：
1. 更新 `docs/STATUS_SNAPSHOT_*.md` 中的"测试工具链状态"小节
2. 更新 `docs/LEGADO_CAPABILITY_INVENTORY.md` 中受影响能力项的"证据"列
3. 如果跑了批量测试，记录通过率到 `reports/tooling/` 并在快照中引用
4. 不得凭单次测试结果声称能力已完成，必须对照 97 项清单逐项验证

### 当前关键断链（截至 2026-06-27）

1. **CLI 无 HTTP 客户端**：`grep "reqwest|ureq|hyper|std::net" tools/reader-cli/src/*.rs` = 0
2. **无链式提取**：现有 `--fixture-vertical` 需一次性喂全部 mock 响应，不提取 URL
3. **无 `--test-source` / `--test-corpus`**：批量测试子命令未实现
4. **459 源批量通过率 = 未知**：从未测试过
5. **17 个 Python 工具全部未跑过**

