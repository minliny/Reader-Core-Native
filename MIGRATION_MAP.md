# 迁移地图

> 记录各平台从独立实现迁移到 Rust Core 的进度。
> 旧迁移/roadmap/handoff 文档均已归档至各仓库 `_archived_planning_2026-06-24/`。

## 整体状态

| 阶段 | 描述 | 状态 | 开始 | 完成 |
|------|------|:----:|------|------|
| 0 | 冻结方向与建立迁移清单 | ✅ | 2026-06-24 | 2026-06-24 |
| 1 | HarmonyOS Rust 地基 | 🟡 | 2026-06-24 | |
| 2 | 统一 C ABI 和三端空壳接入 | 🟡 | 2026-06-24 | |
| 3 | 规则内核和 QuickJS | 🟡 | 2026-06-24 | |
| 4 | 远程阅读 Core 侧纵切 | ✅ | 2026-06-24 | 2026-06-24 |
| 5 | 统一数据库、缓存和进度 | 🟡 | 2026-06-24 | |
| 6 | 补齐规则兼容面 | ⬜ | | |
| 7 | 本地书和扩展能力 | ⬜ | | |
| 8 | 退役重复后端和发布 | ⬜ | | |

阶段 4 的完成范围仅限 Core 侧 smoke：`remote.reading.v1` 覆盖
`source.import` → `book.search` → `book.detail` → `book.toc` →
`chapter.content` → `reading.progress.update`，支持 fixture/inline response
以及 `http.execute` host request/complete 回路。它不代表任何平台 App 已完成
真实网络、WebView 或真机阅读链路。阶段 5 当前只有 in-memory
cache/progress smoke，SQLite 持久化和平台迁移仍在后续阶段。

## 平台迁移进度

### HarmonyOS

| 模块 | 状态 | 备注 |
|------|:----:|------|
| NAPI C++ Shim | 🟡 | Core 侧 `.so` smoke 已通过；App 侧 HAP 集成在 `codex/harmony-napi-runtime`，本文不声明完成 |
| ArkTS Wrapper | 🟡 | App 侧 bridge 独立验证中；本文不声明 device/runtime 完成 |
| HTTP 宿主 Adapter | ⬜ | |
| WebView 宿主 Adapter | ⬜ | |
| TTS 宿主 Adapter | ⬜ | |

### Android

| 模块 | 状态 | 备注 |
|------|:----:|------|
| NativeCoreBridge (JNI) | 🟡 | 已新增 Core 侧 JNI shim 和构建脚本；本地 `.so` 仍需 Android NDK 验证；App 侧 Java/Kotlin loading 待完成 |
| HTTP Transport (OkHttp) | ⬜ | 保留为 transport |
| WebView Adapter | ⬜ | 保留 |
| TTS Adapter | ⬜ | 保留 |
| Room → Rust DB | ⬜ | Core V1 只有 in-memory smoke；durable SQLite/platform migration 待完成 |
| Parser → Rust | ⬜ | |
| RSS → Rust | ⬜ | |
| WebDAV/Sync → Rust | ⬜ | |

### iOS

| 模块 | 状态 | 备注 |
|------|:----:|------|
| XCFramework | 🟡 | Core 侧 staticlib + header smoke 已通过；不声明 App runtime integration |
| ReaderCoreClient.swift | 🟡 | Minimal ABI lifecycle + `core.info` / `runtime.ping` compile/link/runtime smoke 已通过；host adapter 与 App integration 待完成 |
| HTTP Transport (URLSession) | ⬜ | 保留为 transport |
| WebView Login Adapter | ⬜ | 保留 |
| TTS Adapter | ⬜ | 保留 |
| Swift Core → Rust | ⬜ | 最终移除 Swift Core 依赖 |

---

*最后更新: 2026-06-24 | 以 `ARCHITECTURE.md` 为准*
