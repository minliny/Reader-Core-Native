# 滚动集成规则

最高优先级入口：`docs/LOCAL_REPO_MIGRATION_DIRECTIVE.md`

Reader 迁移允许多个分支并行，但每个分支都必须以本地仓库实际代码为事实来源，并且
必须能落到代码、构建和验证。

## Integration lane

| Lane | 分支建议 | 目的 | 验证 |
| --- | --- | --- | --- |
| Rust core foundation | `codex/rust-core-foundation-integration` | C ABI、protocol、runtime、error、host bus | Rust workspace + FFI smoke |
| Rust product core | `codex/rust-product-core-integration` | rule、JS、request、reading、storage、local、RSS、sync | crate tests + CLI fixture |
| iOS integration | `codex/ios-rust-core-integration` | Swift wrapper、URLSession/WKWebView/Keychain/File/TTS adapter | iOS build/smoke/App 证据 |
| Android integration | `codex/android-rust-core-integration` | JNI/Kotlin、OkHttp/WebView/Keystore/SAF/TTS adapter | Android NDK/App 证据 |
| HarmonyOS integration | `codex/harmony-rust-core-integration` | Node-API/ArkTS、Harmony HTTP/WebView/credential/file/TTS adapter | HAP/device 证据 |
| Benchmark/release | `codex/corpus-release-gates` | CLI + 三端 fixture/corpus 一致性 | canonical result hash |

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
2. 旧 Core 代码审计与迁移任务图。
3. Rule/JS/request/reading 核心能力。
4. Storage/local/RSS/sync/TTS 契约。
5. iOS/Android/HarmonyOS wrapper 与 host adapter。
6. CLI + 三端 benchmark。
7. 退役旧业务核心路径。
