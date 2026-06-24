# Migration Map

> 记录各平台从独立实现迁移到 Rust Core 的进度。
> 旧迁移/roadmap/handoff 文档均已归档至各仓库 `_archived_planning_2026-06-24/`。

## 整体状态

| 阶段 | 描述 | 状态 | 开始 | 完成 |
|------|------|:----:|------|------|
| 0 | 冻结方向与建立迁移清单 | ✅ | 2026-06-24 | 2026-06-24 |
| 1 | HarmonyOS Rust 地基 | 🟡 | 2026-06-24 | |
| 2 | 统一 C ABI 和三端空壳接入 | 🟡 | 2026-06-24 | |
| 3 | 规则内核和 QuickJS | 🟡 | 2026-06-24 | |
| 4 | 远程阅读 Core-side 纵切 | ✅ | 2026-06-24 | 2026-06-24 |
| 5 | 统一数据库、缓存和进度 | 🟡 | 2026-06-24 | |
| 6 | 补齐规则兼容面 | ⬜ | | |
| 7 | 本地书和扩展能力 | ⬜ | | |
| 8 | 退役重复后端和发布 | ⬜ | | |

阶段 4 的完成范围仅限 Core-side smoke：`remote.reading.v1` 覆盖
`source.import` → `book.search` → `book.detail` → `book.toc` →
`chapter.content` → `reading.progress.update`，支持 fixture/inline response
以及 `http.execute` host request/complete 回路。它不代表任何平台 App 已完成
真实网络、WebView 或真机阅读链路。阶段 5 当前只有 in-memory
cache/progress smoke，SQLite 持久化和平台迁移仍在后续阶段。

## 平台迁移进度

### HarmonyOS

| 模块 | 状态 | 备注 |
|------|:----:|------|
| NAPI C++ Shim | 🟡 | Core-side `.so` smoke passes; App-side HAP integration is on `codex/harmony-napi-runtime` and is not claimed complete here |
| ArkTS Wrapper | 🟡 | App-side bridge is being validated separately; no device/runtime completion claimed |
| HTTP Host Adapter | ⬜ | |
| WebView Host Adapter | ⬜ | |
| TTS Host Adapter | ⬜ | |

### Android

| 模块 | 状态 | 备注 |
|------|:----:|------|
| NativeCoreBridge (JNI) | 🟡 | Core-side JNI shim and build script added; local `.so` build still needs Android NDK validation; App-side Java/Kotlin loading pending |
| HTTP Transport (OkHttp) | ⬜ | 保留为 transport |
| WebView Adapter | ⬜ | 保留 |
| TTS Adapter | ⬜ | 保留 |
| Room → Rust DB | ⬜ | Core V1 has in-memory smoke only; durable SQLite/platform migration pending |
| Parser → Rust | ⬜ | |
| RSS → Rust | ⬜ | |
| WebDAV/Sync → Rust | ⬜ | |

### iOS

| 模块 | 状态 | 备注 |
|------|:----:|------|
| XCFramework | 🟡 | Core-side staticlib + header smoke passes; App runtime integration not claimed |
| ReaderCoreClient.swift | 🟡 | Minimal ABI lifecycle + `core.info` / `runtime.ping` compile/link/runtime smoke passes; host adapters and App integration pending |
| HTTP Transport (URLSession) | ⬜ | 保留为 transport |
| WebView Login Adapter | ⬜ | 保留 |
| TTS Adapter | ⬜ | 保留 |
| Swift Core → Rust | ⬜ | 最终移除 Swift Core 依赖 |

---

*最后更新: 2026-06-24 | 以 `ARCHITECTURE.md` 为准*
