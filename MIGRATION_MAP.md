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
| 4 | 远程阅读完整纵切 | ⬜ | | |
| 5 | 统一数据库、缓存和进度 | ⬜ | | |
| 6 | 补齐规则兼容面 | ⬜ | | |
| 7 | 本地书和扩展能力 | ⬜ | | |
| 8 | 退役重复后端和发布 | ⬜ | | |

## 平台迁移进度

### HarmonyOS

| 模块 | 状态 | 备注 |
|------|:----:|------|
| NAPI C++ Shim | 🟡 | Core-side `.so` smoke passes; App-side HAP integration is on `codex/harmony-napi-runtime` |
| ArkTS Wrapper | 🟡 | App-side bridge is being validated separately |
| HTTP Host Adapter | ⬜ | |
| WebView Host Adapter | ⬜ | |
| TTS Host Adapter | ⬜ | |

### Android

| 模块 | 状态 | 备注 |
|------|:----:|------|
| NativeCoreBridge (JNI) | ⬜ | |
| HTTP Transport (OkHttp) | ⬜ | 保留为 transport |
| WebView Adapter | ⬜ | 保留 |
| TTS Adapter | ⬜ | 保留 |
| Room → Rust DB | ⬜ | |
| Parser → Rust | ⬜ | |
| RSS → Rust | ⬜ | |
| WebDAV/Sync → Rust | ⬜ | |

### iOS

| 模块 | 状态 | 备注 |
|------|:----:|------|
| XCFramework | 🟡 | Core-side staticlib + header smoke; Swift wrapper typecheck gate |
| ReaderCoreClient.swift | 🟡 | Minimal ABI lifecycle + core.info/runtime.ping smoke; host adapters pending |
| HTTP Transport (URLSession) | ⬜ | 保留为 transport |
| WebView Login Adapter | ⬜ | 保留 |
| TTS Adapter | ⬜ | 保留 |
| Swift Core → Rust | ⬜ | 最终移除 Swift Core 依赖 |

---

*最后更新: 2026-06-24 | 以 `ARCHITECTURE.md` 为准*
