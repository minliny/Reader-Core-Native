# Reader-Core-Native

> **Reader 三平台统一内核 — Rust 实现**
>
> 以 [ARCHITECTURE.md](./ARCHITECTURE.md) 为唯一实施规划来源。

## 快速开始

```bash
# 安装 Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 本机验证
./scripts/check-local.sh

# 本机构建 Rust workspace、FFI release 产物，并运行 Rust/C ABI smoke
./scripts/build-local.sh

# 阶段 1：仅构建 OHOS Rust staticlib
rustup target add aarch64-unknown-linux-ohos
./scripts/build-ohos.sh

# 阶段 1：构建 HarmonyOS NAPI smoke module（需要 DevEco/OHOS SDK）
./scripts/build-harmony-napi.sh
```

OHOS、Android、iOS 平台产物脚本会按 [ARCHITECTURE.md](./ARCHITECTURE.md)
阶段 1/2 补齐；当前 `build-harmony-napi.sh` 验证 Rust staticlib 能链接为
HarmonyOS NAPI `.so`，HAP 集成和真机加载仍需在 HarmonyOS App 仓库完成。

## 目录

- [ARCHITECTURE.md](./ARCHITECTURE.md) — 完整架构与实施规划（**唯一权威文档**）
- [FEATURE_MATRIX.md](./FEATURE_MATRIX.md) — 能力归属表
- [MIGRATION_MAP.md](./MIGRATION_MAP.md) — 各平台迁移进度
- [include/reader_core.h](./include/reader_core.h) — C ABI 头文件
- [protocol/](./protocol/) — JSON 消息协议 Schema

## 仓库关系

```
Reader-Core-Native          ← 此仓库：唯一业务内核（Rust）
Reader-for-iOS              ← UI + Apple Host Adapters
Reader-for-Android          ← UI + Android Host Adapters
Reader-for-HarmonyOS        ← UI + Harmony Host Adapters（首个平台验收目标）
Reader-Core (Swift)         ← 归档参考（冻结新功能）
```

## 旧规划文档

各平台仓库中的旧规划/架构/开发计划等文档已在 2026-06-24 统一归档至各自 `_archived_planning_2026-06-24/` 目录，不再作为实施依据。
