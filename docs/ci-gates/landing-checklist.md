# CI Gates 落地清单（待落地项规格）

> 本文件是 [README.md](./README.md) 第 3 节「待落地」项的细化，供后续分支
> 作为规格输入。本分支（`codex/goal-ci-gate-design`）只产出设计文档，不落地
> 任何 workflow 或脚本。每个 gap 标注：现状、fail-closed 要求、建议归属分支、
> 验收标准、边界声明。

> 基线：`origin/codex/core-product-integration` @ `fb4c3a7`。

## 通用 fail-closed 契约

所有待落地 gate 在环境/依赖缺失时必须满足：

1. **非零退出码**（≥1）。
2. **stderr 明确提示**缺失项与安装方式（参照 `build-ios-xcframework.sh:18-20`
   的 `echo "install with: ..." >&2` 风格）。
3. **禁止**静默跳过、`|| true`、降级为"通过"。
4. **禁止**用 Core-side smoke 冒充 App/真机 proof。

---

## Gap A — Android JNI gate 不存在

| 字段 | 内容 |
| --- | --- |
| 现状 | 基线 `bindings/android/` 仅 `.gitkeep`；无 JNI shim、无 `scripts/build-android*.sh`、无 gate |
| 影响层 | Layer 3 |
| fail-closed 要求 | 落地前：任何 CI job 调用 "Android gate" 必须显式 `exit 1` 并提示 `bindings/android 未实现`。**禁止**用空 `.gitkeep` 路径伪通过 |
| 建议归属分支 | `codex/android-jni-*`（落地 shim + 脚本）；CI 守门归 `codex/ci-android-gate` |
| 验收标准 | (a) 存在 `scripts/build-android-jni.sh`；(b) 缺 NDK 时 `exit 1` + 提示；(c) 产物 `libreader_core_jni.so`（arm64-v8a）存在；(d) JNI smoke 调用 `core.info`/`runtime.ping` 返回正确 ABI 版本 |
| 边界声明 | JNI smoke ≠ App/Compose/UI proof；按 `docs/ROLLING_INTEGRATION.md` 规则，需 clean pushed 分支 + JNI smoke 证据方可集成 |

## Gap B — Harmony HAP / 真机 gate 不存在

| 字段 | 内容 |
| --- | --- |
| 现状 | 基线 `bindings/harmony/` 只有 `native/CMakeLists.txt` + `reader_napi.cpp`；无 README、无 `oh-package.json5`、无 sdk 测试、无 HAP 打包 |
| 影响层 | Layer 2（HAP/真机部分） |
| fail-closed 要求 | NAPI `.so` gate（已存在）止步于产物；任何"HAP gate / 真机 gate"在落地前必须 `exit 1` + 提示 `HAP 打包未实现`。**禁止**用 `.so` 存在冒充真机 parity |
| 建议归属分支 | `codex/harmony-app-*`（HAP/ArkTS）；CI 守门归 `codex/ci-harmony-gate` |
| 验收标准 | (a) HAP 打包脚本存在；(b) 缺 DevEco/签名工具时 `exit 1`；(c) release HAP 真机可加载；(d) 连续 create/destroy runtime 不崩溃；(e) Rust worker 回调 ArkTS 不悬空（对应 `ARCHITECTURE.md` 阶段 1 退出条件） |
| 边界声明 | 严格遵循 `docs/ROLLING_INTEGRATION.md`："Do not move the HarmonyOS lane based on HAP packaging alone. Device/runtime claims require platform-real evidence." |

## Gap C — iOS App / URLSession / WebView gate 不存在

| 字段 | 内容 |
| --- | --- |
| 现状 | 基线 `bindings/ios/` 有 XCFramework + Swift wrapper smoke（`core.info`/`runtime.ping`），但无 App/URLSession/WebView 集成 |
| 影响层 | Layer 1（App 部分） |
| fail-closed 要求 | wrapper smoke（已存在）不得冒充 App proof。任何"iOS App gate"在 host 仓库协同落地前必须 `exit 1` + 提示 `App 集成需 host 仓库` |
| 建议归属分支 | iOS host 仓库（非本仓库）；本仓库 CI 守门归 `codex/ci-ios-app-gate`（仅占位 fail-closed） |
| 验收标准 | (a) App gate 脚本存在；(b) 缺 host 仓库/真机时 `exit 1`；(c) URLSession HTTP transport 走 `http.execute` host 回路；(d) WebView 登录 Cookie 导入 Core |
| 边界声明 | 按 `docs/ROLLING_INTEGRATION.md`："Do not treat iOS Swift wrapper smoke as host adapter or App/device proof." |

## Gap D — OHOS libclang 缺失仅间接闭环

| 字段 | 内容 |
| --- | --- |
| 现状 | `scripts/build-ohos.sh:42` 设 `LIBCLANG_PATH` 默认值 `/Library/Developer/CommandLineTools/usr/lib`，但缺失时未显式 `exit`；bindgen 自身会失败，属间接闭环 |
| 影响层 | Layer 2（OHOS 交叉） |
| fail-closed 要求 | 改为显式闭环：检测 `LIBCLANG_PATH` 指向的 `libclang` 不存在时 `exit 1` + 提示安装方式（CommandLineTools 或 LLVM） |
| 建议归属分支 | `codex/ci-ohos-libclang-guard`（仅改 `scripts/build-ohos.sh`） |
| 验收标准 | (a) `LIBCLANG_PATH` 无效时 `exit 1`；(b) stderr 提示安装方式；(c) 有效时正常交叉编译 |
| 边界声明 | 本分支不落地；仅规格输入 |

---

## 落地顺序建议（依赖优先）

1. **Gap D**（最小、纯脚本守门）— 解除 OHOS 交叉的间接闭环。
2. **Gap A**（Android JNI shim + 脚本）— 填补 Layer 3 空白。
3. **Gap B**（Harmony HAP/真机）— 依赖 NAPI `.so` 已稳定 + DevEco。
4. **Gap C**（iOS App）— 依赖 host 仓库协同，跨仓库协调成本最高。

每项落地单独一个分支、单独一个 commit，遵守各自分支的硬约束边界。

---

## 与 README.md 的对应

| README.md 第 3 节项 | 本文件 gap |
| --- | --- |
| Android JNI gate 不存在 | Gap A |
| Harmony HAP/真机 gate 不存在 | Gap B |
| iOS App/URLSession/WebView gate 不存在 | Gap C |
| OHOS 交叉缺 libclang（部分） | Gap D |

*本文件为设计文档，不修改任何 workflow 或脚本。*
