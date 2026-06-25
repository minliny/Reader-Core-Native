# 滚动集成规则

最高优先级入口：`docs/LOCAL_REPO_MIGRATION_DIRECTIVE.md`

主线执行计划：`docs/MAINLINE_EXECUTION_PLAN.md`

Reader 迁移允许多个分支并行，但每个分支都必须以本地仓库实际代码为事实来源，并且
必须能落到代码、构建和验证。

## Integration lane

| Lane | 分支建议 | 目的 | 验证 |
| --- | --- | --- | --- |
| Rust core foundation | `codex/rust-core-foundation-integration` | C ABI、protocol、runtime、error、host bus | Rust workspace + FFI smoke |
| Legado BookSource compat | `codex/booksource-compat-protocol` | Legado BookSource raw object、unknown field、raw rule 保真和 `source.import` conformance | 已通过 PR #4 合入 |
| Legado DSL executor | `codex/legado-rule-dsl-executor` | 独立实现 Legado CSS 管道链 DSL，不通过 `RuleStepSpec` 硬凑 | 已通过 PR #15 合入 |
| JS runtime compat | `codex/reader-js-compat-runtime` | JS helper/runtime、host callback stub、timeout/cancel 边界 | PR #16 已打开 |
| Request/host contract | `codex/request-host-contract` | charset/cookie/redirect/retry、host replay、conformance | 待开始或恢复 |
| Rust product core | `codex/rust-product-core-integration` | reading、RSS、sync、TTS、更多 storage/local 能力 | crate tests + CLI fixture |
| iOS integration | `codex/ios-rust-core-integration` | Swift wrapper、URLSession/WKWebView/Keychain/File/TTS adapter | Native shell smoke 已通过 PR #12 合入；App/device 仍待证据 |
| Android integration | `codex/android-rust-core-integration` | JNI/Kotlin、OkHttp/WebView/Keystore/SAF/TTS adapter | Native JVM adapter 已通过 PR #2 合入；`.so`/AAR/device 仍待证据 |
| HarmonyOS integration | `codex/harmony-rust-core-integration` | Node-API/ArkTS、Harmony HTTP/WebView/credential/file/TTS adapter | Host PR #2 draft；缺 real-device proof |
| Benchmark/release | `codex/corpus-release-gates` | CLI + 三端 fixture/corpus 一致性 | 工具基础已通过 PR #13 合入；真实三端 run 仍待完成 |

## 分支进入条件

每个分支开始前必须完成：

```bash
pwd
find .. -maxdepth 2 -type d -name .git
git -C <repo> status --short
git -C <repo> branch --show-current
git -C <repo> log -5 --oneline
```

并在报告或提交说明中记录：

- 实际 Rust 目标仓库路径。
- 旧 `Reader-Core` 状态。
- iOS/Android/HarmonyOS 仓库状态。
- 本分支写入范围。
- 本分支不触碰哪些 dirty 文件。

## 合并规则

- 不从 dirty worktree 直接合并，除非 dirty 文件全部属于当前分支并已解释。
- 不覆盖其他 agent 或用户在宿主仓库中的修改。
- 平台 wrapper 不定义业务语义；业务语义回 Rust Core。
- ABI/protocol 变更必须先进入 Rust core foundation lane。
- raw Legado DSL 字符串必须进入 `LegadoBookSource` / `LegadoRuleDsl` / `LegadoRulePipeline`，
  不能进入 `RuleStepSpec`。
- iOS/Android/HarmonyOS 只能声明自己真实验证过的层级。
- wrapper smoke 不等于 App/device proof。
- 静态报告不等于可运行结果。

## 分支合并前必须回答

1. 读取了哪些本地仓库代码路径？
2. 从旧 `Reader-Core` 迁移了什么，或为什么不迁移？
3. 修改了哪些 Rust Core crate、ABI 或 protocol？
4. iOS、Android、HarmonyOS 哪些 adapter 需要跟进？
5. 跑了哪些构建/测试命令？
6. 是否产生可运行结果？
7. 是否能进入跨平台 fixture/corpus benchmark？

## 推荐集成顺序

1. Rust ABI/protocol/runtime。
2. Legado BookSource 兼容入口与旧 Core BookSource 迁移资产对齐。
3. Legado DSL executor 最小 search/detail/toc/content 闭环（已合入）。
4. JS helper/runtime 兼容（PR #16 已打开）。
5. request descriptor / host capability 契约。
6. Storage/local/RSS/sync/TTS 契约（storage/local fixture gates 已合入，RSS/sync/TTS 待补）。
7. iOS/Android/HarmonyOS wrapper 与 host adapter（Native smoke 已部分合入，真实 App/device 待补）。
8. CLI + 三端 benchmark（工具已合入，真实三端 run 待补）。
9. 退役旧业务核心路径。
