# 三端迁移地图

最高优先级入口：`docs/LOCAL_REPO_MIGRATION_DIRECTIVE.md`

本文记录 iOS、Android、HarmonyOS 从现有实现迁移到统一 Rust Reader-Core 的路线。

## 当前本地仓库状态

最近一次本地扫描：2026-06-25。后续 agent 仍必须重新执行开工前安全检查，本表只记录
本次 checkpoint 的事实快照。

| 仓库 | 当前分支 | 状态 |
| --- | --- | --- |
| `Reader-Core` | `main` | clean，旧核心迁移源；最新提交 `a6db53e0 docs: add Reader-Core to Rust migration ledger` |
| `legado` | `master` | clean，只读 Legado 兼容语义基线；最新提交 `da17bb2be 优化 #5784` |
| `Reader for iOS` | `codex/ios-rust-host-adapter` | clean，iOS 宿主迁移目标；最新提交 `7dabaae Update STATUS.md with Round 6 commit hash` |
| `Reader for Android` | `main` | clean，Android 宿主迁移目标 |
| `Reader for HarmonyOS` | `codex/harmony-napi-runtime` | clean，HarmonyOS 宿主迁移目标；最新提交 `5a69ed1 docs: record HarmonyOS HostBus simulator evidence` |
| Rust 目标仓库 | `codex/full-branch-directory-consolidation` | clean，当前 checkpoint base 为 `Reader-Core-Native`；BookSource 纠偏在 `codex/booksource-compat-protocol` / PR #4 |

## 阶段状态

| 阶段 | 描述 | 状态 |
| --- | --- | :---: |
| 0 | 本地仓库定位、安全检查、dirty 状态记录 | 已完成本轮检查 |
| 1 | 旧 `Reader-Core` 实际代码审计 | 待系统化 |
| 2 | Rust C ABI / protocol / runtime 边界冻结 | 部分完成 |
| 3 | Rule/JS/request/reading 核心能力迁移 | 部分完成 |
| 4 | SQLite/cache/sync/local/RSS/TTS 契约迁移 | 部分完成 |
| 5 | iOS strangler migration | 待 App-side 验证 |
| 6 | Android strangler migration | 待 App-side 验证 |
| 7 | HarmonyOS strangler migration | 待 HAP/device 验证 |
| 8 | 三端 corpus/fixture 一致性 benchmark | 待建设 |
| 9 | 退役旧业务核心路径 | 未开始 |

## iOS 迁移

| 模块 | 迁移方向 | 状态 |
| --- | --- | --- |
| Swift 旧 Core 调用 | 切到 Rust C ABI + Swift wrapper | 部分完成 |
| URLSession transport | 作为 `http.execute` host adapter | 待 App-side 验证 |
| WKWebView 登录/Cookie | 保留在 iOS adapter，结果回传 Core | 待迁移 |
| Keychain | 平台 credential store | 待契约化 |
| File picker / sandbox | 平台 adapter | 待迁移 |
| TTS | 平台播放，Core 提供数据/指令契约 | 待迁移 |
| Reader UI | 保留 SwiftUI | 不进入 Rust Core |

## Android 迁移

| 模块 | 迁移方向 | 状态 |
| --- | --- | --- |
| JNI/Kotlin wrapper | 消费 Rust C ABI | 部分完成 |
| OkHttp transport | 作为 `http.execute` host adapter | 待 App-side 验证 |
| WebView/CookieManager | 平台 adapter | 待迁移 |
| Keystore | 平台 credential store | 待契约化 |
| SAF/File | 平台 adapter | 待迁移 |
| Room/本地数据 | 业务语义迁到 Rust SQLite，平台只提供目录/权限 | 待迁移 |
| TTS/UI | 平台负责 | 待接入契约 |

## HarmonyOS 迁移

| 模块 | 迁移方向 | 状态 |
| --- | --- | --- |
| Node-API/ArkTS wrapper | 消费 Rust C ABI | 部分完成 |
| Harmony HTTP | 作为 `http.execute` host adapter | 待验证 |
| WebView/session | 平台 adapter | 待迁移 |
| Credential store | 平台 adapter | 待契约化 |
| 文件/目录权限 | 平台 adapter | 待迁移 |
| HAP/device smoke | 证明真实平台可加载 Rust Core | 待完成 |
| UI/TTS | 平台负责 | 待接入契约 |

## 迁移完成标准

- 三端创建同一个 Rust runtime。
- 三端使用同一 C ABI/protocol。
- 三端同一 fixture/corpus 输出同一 canonical result。
- 旧 `Reader-Core` 对应能力已迁移、废弃或标注为平台 adapter。
- 旧业务核心路径从发布链路中退役。
