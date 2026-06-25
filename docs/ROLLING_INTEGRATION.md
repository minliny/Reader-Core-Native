# 滚动集成

Reader-Core-Native 使用滚动集成来支撑多个 agent 并行开发。不要等待一批分支全部
结束后再前进；每个完成分支都应合入能验证它的最小 integration lane。

当前更高层的目标和长期分支拆分见 `docs/FULL_DEVELOPMENT_ROADMAP.md`。本文只描述
如何把已完成工作安全接入集成分支。

## Integration lane

| Lane | 分支 | 目的 | 等待条件 |
| --- | --- | --- | --- |
| Core foundation | `codex/core-foundation-integration` | protocol、runtime、rule、QuickJS、ABI smoke | 只等已完成 foundation 分支 |
| Core product | `codex/core-product-integration` | remote reading、content、storage、progress | foundation 加已完成 product 分支 |
| Android | `codex/android-integration` | JNI bridge 与 Android host smoke | product baseline 加 Android JNI 分支 |
| iOS | `codex/ios-integration` | Swift wrapper runtime smoke 与 iOS host proof | product baseline 加 iOS wrapper/runtime 分支 |
| HarmonyOS | `codex/harmony-integration` | NAPI、ArkTS bridge、HAP proof | product baseline 加 Harmony 白名单变更 |

当前 `codex/full-branch-directory-consolidation` 已经把此前散落分支合并到一个基线。
后续新 work 应优先从这个基线或用户指定的新 integration base 开始。

## 当前 Core-side 基线

当前基线已包含：

- `remote.reading.v1`
- `http.execute`
- source import、search、detail、TOC、chapter content、progress update
- fixture/inline response 与 host request/complete 回路
- C ABI lifecycle、status、last-error、panic guard
- iOS/Android/Harmony wrapper smoke 或 wrapper shape
- local TXT、RSS、storage、sync 基础数据层

注意：这些仍然主要是 Core-side 或 wrapper-side 证据，不等于平台 App/device 完成。

## 集成命令模式

当一个分支准备好时，使用 `scripts/integration-queue.sh`：

```bash
scripts/integration-queue.sh \
  codex/android-integration \
  origin/codex/full-branch-directory-consolidation \
  origin/codex/<android-jni-branch>
```

可选 gate：

```bash
RUN_OHOS=1 RUN_NAPI=1 PUSH=1 scripts/integration-queue.sh \
  codex/harmony-integration \
  origin/codex/full-branch-directory-consolidation \
  origin/codex/<harmony-app-integration-branch>
```

## 规则

- 不从 dirty worktree 集成。
- 每个 integration lane 使用独立 worktree。
- 按依赖顺序合并 source branch。
- 遇到冲突或 gate 失败时先停下，在 integration 分支修复；只有 source branch 自身
  错误时才回源头修改。
- HarmonyOS 不能只凭 HAP packaging 声明 device/runtime parity。
- 不合并未经审计的 HarmonyOS 仓库级 dirty changes；只 stage 任务拥有的
  NAPI/ArkTS/HAP 文件。
- 不用本地 placeholder branch 声明 Android JNI 完成；需要 clean pushed branch 和
  JNI smoke evidence。
- iOS Swift wrapper smoke 不等于 URLSession/WebView/App 集成证明。

## 分支合并前必须回答的问题

每个集成 PR 或合并提交都要回答：

1. 关闭了哪条 Legado capability？
2. 迁移、回放、host 化或归档了哪个旧 Reader-Core asset？
3. 修改了哪个 Native/C ABI contract？
4. 哪些 platform wrapper 必须更新？
5. 哪个 corpus benchmark case 能证明三端 canonical result 一致？

如果第 5 项答案是“没有”，该分支可以作为 infrastructure 合并，但不能计入 Legado
parity closure。

## 下一步触发器

更细的长期路线见 `docs/FULL_DEVELOPMENT_ROADMAP.md`。短期滚动触发器：

- Legado 能力账本完成后，所有实现分支要绑定 capability row。
- Reader-Core 迁移账本完成后，所有实现分支要绑定 migrate/replay/host/archive
  决策。
- Corpus runner 一旦可用，feature 分支必须尽量附带 canonical DTO/hash evidence。
- Platform wrapper 分支只证明消费 ABI，不定义 Core 语义。
