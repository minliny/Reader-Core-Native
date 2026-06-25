# 发布 readiness 证据

> 当前最高优先级文档是 `docs/LOCAL_REPO_MIGRATION_DIRECTIVE.md`。本文是历史
> release evidence 记录，不覆盖当前“以 Rust 为唯一业务内核”的迁移路线。

> 长期 goal 分支：`codex/goal-release-evidence`
> 基线：`origin/codex/core-product-integration`（首轮回采 sha `fb4c3a7`）
> 目的：为 Reader-Core-Native 建立**发布 readiness 证据包**，覆盖命令矩阵、
> 平台矩阵、阻塞项、通过标准、未验证项、环境依赖。

## 作用域与硬约束

- **只允许新增和修改 `evidence/release-readiness/**`。**
- 禁止修改：`.github/**`、`scripts/**`、`crates/**`、`bindings/**`、`protocol/**`、
  `Cargo.*`、顶层产品文档。
- 本证据包**不改任何实现**。只运行只读验证命令并记录输出摘要。
- 每轮记录：验证日期、命令、平台、结果、失败原因、是否可作为 release gate。
- **禁止把 smoke 通过写成 App/device 通过。** Core-side smoke（host FFI、host sim
  wrapper、Core-side 交叉产物构建）与 App/device 真机验证必须分开陈述。
- 每轮提交一个 commit。

## 目录

| 文件 | 内容 |
|------|------|
| [command-matrix.md](command-matrix.md) | 验证命令矩阵：命令、作用、平台、gate 状态 |
| [platform-matrix.md](platform-matrix.md) | 平台矩阵：每平台已验证 / 未验证项 |
| [pass-criteria.md](pass-criteria.md) | 通过标准与 release gate 定义 |
| [blockers.md](blockers.md) | 阻塞项清单 |
| [unverified.md](unverified.md) | 未验证项清单 |
| [environment.md](environment.md) | 环境依赖（工具链、SDK、target） |
| [rounds/](rounds/) | 每轮验证日志（按日期编号） |

## 当前结论（截至 round-01）

- **Core-side smoke release gate**：全部绿（macOS host 上 fmt / test / conformance /
  vertical / ffi-smoke / iOS xcframework+wrapper / OHOS+NAPI .so）。
- **App/device release gate**：未满足。仅 Core-side 产物构建与 host smoke 通过，
  不等于 App/真机通过。
- **Android**：无 Core-side 产物构建路径（`bindings/android` 仅 `.gitkeep`，无构建
  脚本），Android release readiness 暂不可声明。

## 如何追加一轮

1. 在 `rounds/` 下新建 `YYYY-MM-DD-round-NN.md`，NN 为两位递增编号。
2. 按“验证日期 / 命令 / 平台 / 结果 / 失败原因 / 是否可作为 release gate”逐条记录。
3. 如实区分 smoke 与 App/device；如命令失败，记录失败原因与退出码。
4. 必要时更新 `command-matrix.md`、`platform-matrix.md`、`blockers.md`、
   `unverified.md` 的状态。
5. 仅在 `evidence/release-readiness/**` 内改动，提交一个 commit。
