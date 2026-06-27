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
`docs/MAINLINE_EXECUTION_PLAN.md` > 其他 roadmap / 审计 / 报告 > 历史归档文档。
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
