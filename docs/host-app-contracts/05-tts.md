# 05：TTS（文本切片 / 播放队列 vs 系统发声）

> 主题域：TTS 文本切片、播放队列状态机、章节边界模型 vs 系统发声归属。
> 状态：🟡 契约已立（未实现）。本文件不声明任何实现完成。

## 1. 范围

**覆盖：**

- 章节正文 → 可发声切片（slice）的切分策略与产物模型。
- 播放队列状态机：`Idle / Playing / Paused / Completed / Stopped` 与
  per-slice 生命周期 `Pending / Speaking / Done / Skipped / Failed`。
- 章节边界模型：当前章节、下一章节预取、队列耗尽时的 drain 行为。
- Core 与 host 在 TTS 链路上的职责切分：Core 拥有切片与队列语义，
  host 拥有系统发声引擎与平台音频会话。

**不覆盖（归其它主题域或 Out-of-scope）：**

- 系统发声引擎本身（`AVSpeechSynthesizer` / `Android TextToSpeech` /
  HarmonyOS `@ohos.textToSpeech`）→ **Host-owned**，Core 不接触。
- 音频会话/打断/路由切换（蓝牙耳机、静音模式、电话打断恢复）→
  **Host-owned**，V1 不契约化。
- 语音参数（音色、语速、音高、语言选择）→ **Host-owned**（V1）；
  Core 仅提供切片文本，不指定发音参数。
- 章节正文获取（fetch / 解码 / 规则解析）→ 01 network/session 与
  04 local book/files。
- 阅读进度同步（TTS 播放进度回写）→ 02 local storage/sync。
- 字幕高亮同步（slice 与段落/字符 offset 的 UI 映射）→ 06 ui/background。
- SSML / 音素 / 多说话人 / 情感标注 → **Out-of-scope**（V1 不交付）。

**上游事实来源：**

- `crates/reader-contract/src/tts.rs`（Rust DTO 真相源，本文件立约时已建）。
- `protocol/reader-command.schema.json` §`$defs/Tts*`（command 参数契约）。
- `protocol/reader-event.schema.json` §`$defs/Tts*Data`（event 结果契约）。
- `FEATURE_MATRIX.md` TTS 行（能力归属总表）。

**参考来源（迁移源仓库，仅用于设计参照，不作契约来源）：**

- `Documents/legado/app/src/main/java/io/legado/app/help/TTS.kt`：
  Android `TextToSpeech` 薄封装，按 `\n` 切分，`QUEUE_ADD` 入队，
  utteranceId = `tag + index`。证实"切片 + 入队"是最小可用模型。
- `Documents/Reader Core/Sources/ReaderCoreProtocols/HostCapabilityProtocols.swift`：
  Swift 版 `SpeechUtteranceRequest` / `SpeechPlaybackHandle` /
  `TextToSpeechAdapter` 协议。已有"Core 提供 DTO、host 发声"的 handoff
  意图，但无切片/队列/边界抽象——本文件补齐该层。

## 2. Capability inventory

| 子能力 | 归属类别 | 当前事实来源 |
|--------|----------|--------------|
| 章节正文切分为可发声 slice | **Core-owned** | `tts.rs` `TtsSlicePlan`、schema `TtsSliceParams` |
| 切分策略选择（paragraph / sentence / paragraph-then-sentence / line-break） | **Core-owned** | `tts.rs` `TtsSlicingStrategy` |
| slice 字符 offset 与段落索引（供 UI 高亮同步） | **Core-owned** | `tts.rs` `TtsSlice.{charStart,charEnd,paragraphIndex}` |
| 播放队列状态机（Idle/Playing/Paused/Completed/Stopped） | **Core-owned** | `tts.rs` `TtsQueueState`、`TtsQueueSnapshot` |
| per-slice 生命周期（Pending/Speaking/Done/Skipped/Failed） | **Core-owned** | `tts.rs` `TtsSliceStatus` |
| 章节边界迁移计划（current / next / drainBehavior） | **Core-owned** | `tts.rs` `TtsChapterTransition` |
| 队列耗尽时是否自动进入下一章 | **Core-owned**（策略）+ **Host-owned**（执行触发） | `tts.rs` `TtsQueueDrainBehavior` |
| 系统发声引擎调用 | **Host-owned** | 平台 OS API |
| 语音参数（音色/语速/音高/语言） | **Host-owned**（V1） | 本文件立约（V1 不契约化） |
| 音频会话/打断/路由 | **Host-owned** | 本文件立约（V1 不契约化） |
| 字幕高亮（slice → UI 段落映射） | **Shared-contract**（Core 提供 offset，host 渲染） | 06 ui/background |
| TTS 进度回写阅读进度 | **Shared-contract**（host 上报 slice index，Core 折算 chapterProgress） | 02 local storage/sync |
| SSML / 音素 / 多说话人 | **Out-of-scope** | V1 不交付 |

## 3. Contracts

下列契约已在 `protocol/reader-command.schema.json` 与
`protocol/reader-event.schema.json` 落地（`$defs/Tts*`），并由
`crates/reader-contract/src/tts.rs` 提供 Rust DTO。本节为契约的语义说明，
权威定义以 schema 与 Rust 类型为准。

### 3.1 `tts.slice` — 文本切片（Core → host 请求，Core 计算并返回 plan）

**Command params（`TtsSliceParams`）：**

```json
{
  "chapter": {
    "sourceId": "src-1",
    "bookId": "book-1",
    "chapterIndex": 0,
    "chapterTitle": "第一章",
    "chapterUrl": "https://books.example.test/ch1"
  },
  "content": "第一段内容。\n第二段内容。",
  "strategy": "paragraph"
}
```

- `chapter`：章节引用（`TtsChapterRef`），复用 remote-reading 的章节身份，
  便于 TTS 跨 TOC 推进时无需重新拉取。
- `content`：章节正文（已由 `chapter.content` 命令提取并规范化）。
  必须非空（`minLength: 1`）。
- `strategy`：切分策略，默认 `paragraph`。见 `TtsSlicingStrategy` enum。

**Result data（`TtsSliceData`，event）：**

```json
{
  "plan": {
    "chapter": { "sourceId": "src-1", "bookId": "book-1", "chapterIndex": 0 },
    "strategy": "paragraph",
    "slices": [
      { "index": 0, "text": "第一段内容。", "charStart": 0, "charEnd": 6, "paragraphIndex": 0 },
      { "index": 1, "text": "第二段内容。", "charStart": 7, "charEnd": 13, "paragraphIndex": 1 }
    ],
    "sourceCharCount": 13
  }
}
```

- `slices[].text`：非空规范化文本，host 直接喂给系统 TTS 引擎。
- `slices[].charStart` / `charEnd`：半开区间，指向原始 `content`，
  供 UI 高亮同步（06 ui/background）。
- `slices[].paragraphIndex`：段落索引，用于"逐段高亮"UI 模式。
- `sourceCharCount`：原始正文总字符数，用于进度折算。

**归属：** Core-owned。host 不参与切分逻辑，只消费 `slices`。

### 3.2 `tts.queue.status` — 播放队列状态机（Core → host 查询）

**Command params（`TtsQueueStatusParams`）：**

```json
{
  "chapter": { "sourceId": "src-1", "bookId": "book-1", "chapterIndex": 0 }
}
```

**Result data（`TtsQueueStatusData`，event）：**

```json
{
  "snapshot": {
    "state": "playing",
    "currentSliceIndex": 2,
    "totalSlices": 13,
    "completedSlices": 2,
    "chapter": { "sourceId": "src-1", "bookId": "book-1", "chapterIndex": 0 },
    "sliceStatuses": ["done", "done", "speaking", "pending", "..."]
  }
}
```

- `state`：`idle / playing / paused / completed / stopped`。
  - `idle`：无 plan 加载。
  - `playing` / `paused`：host 发声引擎正在发声 / 暂停。
  - `completed`：当前章节所有 slice 已 `done`。
  - `stopped`：被显式停止（非自然结束）。
- `currentSliceIndex`：`null` 当 `state=idle`；否则为当前发声 slice 的索引。
- `completedSlices`：终态 slice 计数（`done + failed + skipped`）。
- `sliceStatuses`：与 `TtsSlicePlan.slices` 平行的状态数组。V1 可省略
  （`default: []`），仅返回 `state + currentSliceIndex + counts`。

**归属：** Core 拥有状态机语义；host 驱动状态迁移（通过后续
`tts.queue.*` 控制命令上报，V1 仅契约化查询，控制命令待后续 owner 评估）。

**Gap F（队列控制命令缺失）：** V1 仅契约化 `tts.queue.status`（查询），
未契约化 `tts.queue.play` / `tts.queue.pause` / `tts.queue.resume` /
`tts.queue.stop` / `tts.queue.next` / `tts.queue.prev` 等控制命令。
host 如何把系统 TTS 引擎的 utterance 回调映射到 `TtsSliceStatus` 迁移
也未契约化。→ 后续 owner: protocol schema + Core runtime。

### 3.3 `tts.chapter.plan` — 章节边界迁移（Core → host 查询）

**Command params（`TtsChapterPlanParams`）：**

```json
{
  "chapter": { "sourceId": "src-1", "bookId": "book-1", "chapterIndex": 0 },
  "drainBehavior": "advance-to-next"
}
```

**Result data（`TtsChapterPlanData`，event）：**

```json
{
  "transition": {
    "current": { "sourceId": "src-1", "bookId": "book-1", "chapterIndex": 0 },
    "next": { "sourceId": "src-1", "bookId": "book-1", "chapterIndex": 1 },
    "drainBehavior": "advance-to-next"
  }
}
```

- `current`：当前章节引用。
- `next`：下一章节引用，`null` 表示当前已是末章。
  host 可据此预取下一章 `chapter.content` 并预跑 `tts.slice`，实现无缝衔接。
- `drainBehavior`：
  - `stop-on-boundary`（默认）：当前章节队列耗尽后停止，不自动进入下一章。
  - `advance-to-next`：当前章节队列耗尽后自动加载下一章 plan 并继续发声。

**归属：** Core 拥有边界决策（基于 TOC）；host 负责预取与发声衔接执行。

**Gap G（预取契约未定义）：** `next` 仅提供章节引用，host 预取下一章
`chapter.content` + `tts.slice` 的触发时机与失败回退策略未契约化。
→ 后续 owner: Core runtime + protocol schema。

### 3.4 Core / Host 职责切分总表

| 阶段 | Core | Host |
|------|------|------|
| 章节正文获取 | `chapter.content` 提取并规范化 | 经 `http.execute` 透传响应字节 |
| 切片 | `tts.slice` 计算 `TtsSlicePlan` | 不参与 |
| 队列状态 | `tts.queue.status` 维护 `TtsQueueSnapshot` | 上报发声进度（V1 未契约化） |
| 章节边界 | `tts.chapter.plan` 计算 `TtsChapterTransition` | 预取下一章（V1 未契约化） |
| 系统发声 | 不参与 | 调用平台 TTS 引擎，按 slice 顺序发声 |
| 语音参数 | 不参与 | 选择音色/语速/音高/语言 |
| 音频会话 | 不参与 | 管理打断/路由/后台播放 |
| 进度回写 | 折算 `chapterProgress` 并存储（02） | 上报当前 slice index（V1 未契约化） |

## 4. 验收证据要求

> 以下为 *契约成立所需的证据*，不是实现完成的声明。

1. **Conformance fixture**：`protocol/fixtures/conformance/commands/` 下存在
   覆盖 `tts.slice` / `tts.queue.status` / `tts.chapter.plan` 的 valid +
   invalid fixture，且被 `protocol-schema-lint` 验证通过（valid 通过 schema
   校验，invalid 被拒绝）。**已满足**（本立约轮交付 3 valid + 4 invalid）。
2. **Schema↔Contract 一致性**：`reader-contract` crate 内存在测试断言
   `methods::TTS_*` 常量、schema `$defs/Tts*`、Rust DTO 三者一致。
   **已满足**（4 个 schema 一致性测试 + 4 个 DTO round-trip 测试）。
3. **切片行为证据**：一条用例展示给定章节正文 + `strategy=paragraph`，
   Core 产出 `TtsSlicePlan`，其 `slices` 数量、`charStart/charEnd` 半开区间、
   `paragraphIndex` 与原文段落对齐。→ 后续 owner: Core runtime。
4. **队列状态机证据**：一条用例展示 `Idle → Playing → Paused → Playing →
   Completed` 的状态迁移，`completedSlices` 与 `sliceStatuses` 终态一致。
   → 后续 owner: Core runtime。
5. **章节边界证据**：一条用例展示 `drainBehavior=advance-to-next` 时，
   当前章节末 slice 完成后 `TtsChapterTransition.next` 被加载为新的
   `current`；`stop-on-boundary` 时队列进入 `Stopped`。→ 后续 owner:
   Core runtime。
6. **三端发声 handoff 证据**：三端 host adapter 各提供一条冒烟日志，展示
   接收 `TtsSlicePlan` 后调用平台 TTS 引擎按 slice 顺序发声，且不修改
   slice 文本。→ 后续 owner: iOS / Android / Harmony adapter。
7. **V1 能力边界证据**：`core.info` 的 `capabilities` 数组 **不含** TTS
   相关 capability（TTS 是方法而非 capability），`V1_CAPABILITIES` 长度
   不变。**已满足**（本立约轮未新增 capability）。

## 5. Risks

- **队列控制命令缺口（Gap F）**：V1 仅契约化查询，host 无法通过协议
  控制 play/pause/stop/next/prev。若各端自行实现控制路径，会出现"Core
  状态机与 host 发声引擎状态分裂"。缓解：V1 host 可先用平台原生控制，
  Core 状态机仅在下次 `tts.queue.status` 查询时收敛；但跨端一致性靠
  后续 Gap F 闭环。
- **发声进度上报缺口**：host 系统TTS 的 utterance 回调（start/done/error）
  如何映射到 `TtsSliceStatus` 迁移未契约化。若 host 不上报，Core 的
  `TtsQueueSnapshot` 将停留在查询时刻的快照，无法实时反映发声进度。
  → 后续 owner: protocol schema（Gap F 的一部分）。
- **切片粒度与平台 TTS 限制冲突**：某些平台 TTS 引擎对单次 utterance
  长度有上限（如 Android `TextToSpeech` 对超长文本可能截断或失败）。
  Core 的 `paragraph` 策略可能产出超长 slice。缓解：Core 应在 slicer
  内加长度上限（V1 未实现，标记为后续 owner: Core runtime）。
- **预取失败回退（Gap G）**：`advance-to-next` 模式下，若下一章
  `chapter.content` 预取失败，host 应回退到 `stop-on-boundary` 行为还是
  报错？未契约化。→ 后续 owner: Core runtime + protocol schema。
- **章节边界与 TOC 一致性**：`TtsChapterTransition.next` 依赖 TOC 的
  章节顺序。若 TOC 在 TTS 播放过程中被刷新（新增/重排章节），`next`
  可能失效。缓解：Core 应在 `tts.chapter.plan` 时锁定 TOC 快照版本。
- **多端语音参数漂移**：V1 不契约化语音参数，三端用户各自的音色/语速
  设置互不同步。这是有意的（V1 Out-of-scope），但应在文档中显式告知
  用户"TTS 语音设置跟随平台系统设置"。

## 6. Follow-up owners

| 后续工作 | 责任方 |
|----------|--------|
| 实现 `tts.slice` slicer（paragraph / sentence / line-break 策略 + 长度上限） | Core runtime owner |
| 实现 `tts.queue.status` 状态机（维护 `TtsQueueSnapshot`，处理 host 上报） | Core runtime owner |
| 实现 `tts.chapter.plan` 边界计算（基于 TOC 快照） | Core runtime owner |
| 契约化队列控制命令 `tts.queue.play/pause/resume/stop/next/prev`（Gap F） | protocol schema owner |
| 契约化发声进度上报（utterance 回调 → `TtsSliceStatus` 迁移）（Gap F） | protocol schema owner |
| 契约化预取失败回退策略（Gap G） | protocol schema + Core runtime |
| iOS `TextToSpeechHostAdapter`（`AVSpeechSynthesizer`，消费 `TtsSlicePlan`） | iOS adapter owner |
| Android `TextToSpeechHostAdapter`（`Android.TextToSpeech`，消费 `TtsSlicePlan`） | Android adapter owner |
| Harmony `TextToSpeechHostAdapter`（`@ohos.textToSpeech`，消费 `TtsSlicePlan`） | Harmony adapter owner |
| TTS 进度回写阅读进度（跨 02/05） | Core runtime + 02 local storage/sync |
| 字幕高亮同步（slice → UI 段落映射）（跨 05/06） | 06 ui/background |

---

*本文件立约于 `codex/tts-contract-model`，基线 `origin/main`（2ca3454）。
不声明实现完成。*
