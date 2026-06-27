# 测试验证工具链与书源管理规划

日期：2026-06-27

本文是 `docs/PROJECT_CHARTER.md` 红线 1（能力底线 = Legado）的落地执行方案。
定义完整的测试工具链、书源管理工具、评估规则和 CI 流程，可在本地或 GitHub Action 实施。

---

## 1. 问题诊断

### 1.1 当前缺口

| 缺口 | 影响 |
| --- | --- |
| 无批量书源测试 | 3 个手工源覆盖 0.65%，459 源集合从未批量验证 |
| conformance 测格式不测能力 | 173 用例全绿 ≠ 真实书源能跑通 |
| fixture_vertical 是离线回放 | 静态录制响应，不验证网络/JS/编码变化 |
| 17 个工具脚本全部未跑过 | corpus/evidence/benchmark/gate 工具存在但无产出 |
| 无评估规则 | "跑通"无量化定义，无通过率阈值 |
| 无四端 corpus benchmark | S7 空白，四端连同一 Core commit 都没用过 |
| blocker 手工维护 | 53 条手工 JSON，不自动从测试结果生成 |

### 1.2 现有可复用基础

| 资产 | 位置 | 状态 |
| --- | --- | --- |
| `reader-cli --conformance` | tools/reader-cli | ✅ 每轮跑，173 用例 |
| `reader-cli --fixture-vertical` | tools/reader-cli | ✅ 每轮跑，6 真实源 |
| `reader-cli --host-replay` | tools/reader-cli | ✅ 每轮跑，5 用例 |
| `fixture_manifest.py` | tools/fixture-manifest | ❌ 未跑 |
| `corpus_batch_selector.py` | tools/corpus-batch-selector | ❌ 未跑 |
| `corpus_real_run_collector.py` | tools/corpus-real-run-collector | ❌ 未跑 |
| `cross_platform_diff.py` | tools/cross-platform-diff | ❌ 未跑 |
| `evidence_indexer.py` | tools/evidence-indexer | ❌ 未跑 |
| `capability_catalog.py` | tools/capability-catalog | ❌ 未跑（有产出文件） |
| `gate_declaration.py` | tools/gate-declaration | ❌ 未跑 |
| `platform_evidence_validator.py` | tools/platform-evidence-validator | ❌ 未跑 |
| `release_blocker_register.py` | tools/release-blocker-register | ❌ 未跑 |
| `benchmark_run_packager.py` | tools/benchmark-run-packager | ❌ 未跑 |
| 459 源集合 | Reader-Core/临时导入书源-墨辰.txt | ❌ 未用于测试 |
| CI workflow | .github/workflows/core.yml | ⚠️ 只跑 fmt+test+conformance |

---

## 2. 体系架构

```
┌─────────────────────────────────────────────────────────┐
│                    书源管理工具                            │
│  corpus-manager (新增)                                    │
│  ├── 导入: 459 源集合 / Legado 默认源 / 用户自选源          │
│  ├── 分类: 按规则形态(CSS/XPath/JSON/JS/MultiRule/Regex)  │
│  ├── 脱敏: 自动检测 token/cookie → placeholder            │
│  ├── 录制: 对每个源录制搜索/详情/目录/正文响应              │
│  └── 索引: 生成 corpus-manifest.json                      │
├─────────────────────────────────────────────────────────┤
│                    测试工具链                              │
│  ├── L1 单元测试 (cargo test)           — 已有            │
│  ├── L2 协议一致性 (conformance)        — 已有            │
│  ├── L3 真实书源纵切 (fixture-vertical) — 已有(3个)       │
│  ├── L4 批量书源回归 (corpus-batch)     — 新增            │
│  ├── L5 真实网络冒烟 (live-smoke)       — 新增            │
│  └── L6 四端 corpus benchmark           — 新增(S7)        │
├─────────────────────────────────────────────────────────┤
│                    评估规则                                │
│  ├── 通过定义: 5 级(导入/搜索/详情/目录/正文)              │
│  ├── 证据分层: unit/conformance/fixture/batch/live/device │
│  ├── Gate 规则: 各级退出码 + blocker 自动生成             │
│  └── 评估报告: evidence-index + capability-catalog        │
├─────────────────────────────────────────────────────────┤
│                    CI 流程                                 │
│  ├── PR gate: L1+L2+fmt+clippy                           │
│  ├── main gate: L1+L2+L3+L4(抽样)+evidence               │
│  ├── nightly: L4 全量+L5 冒烟+capability-catalog          │
│  └── release: L6 四端 corpus benchmark                    │
└─────────────────────────────────────────────────────────┘
```

---

## 3. 书源管理工具 (corpus-manager)

### 3.1 目标

把 459 源集合变成可管理、可分类、可批量测试的 corpus。

### 3.2 工具设计

**新增工具**: `tools/corpus-manager/corpus_manager.py`

#### 子命令

```bash
# 导入源集合到 corpus
python3 tools/corpus-manager/corpus_manager.py import \
  --from "/Users/minliny/Documents/Reader-Core/临时导入书源-墨辰.txt" \
  --to tests/fixtures/corpus/sources/

# 分析每个源的规则形态，生成分类索引
python3 tools/corpus-manager/corpus_manager.py classify \
  --sources tests/fixtures/corpus/sources/ \
  --out tests/fixtures/corpus/corpus-manifest.json

# 脱敏检测（扫描 token/cookie/apikey）
python3 tools/corpus-manager/corpus_manager.py sanitize \
  --sources tests/fixtures/corpus/sources/ \
  --report tests/fixtures/corpus/sanitize-report.json

# 录制真实响应（对每个源发起搜索请求，录制响应）
python3 tools/corpus-manager/corpus_manager.py record \
  --sources tests/fixtures/corpus/sources/ \
  --keyword "斗破苍穹" \
  --out tests/fixtures/corpus/recorded/ \
  --timeout 10 \
  --max-sources 50  # 先抽样 50 个

# 生成 fixture-vertical 格式（从录制数据）
python3 tools/corpus-manager/corpus_manager.py package \
  --recorded tests/fixtures/corpus/recorded/ \
  --out tests/fixtures/remote_source/

# 验证 corpus 完整性
python3 tools/corpus-manager/corpus_manager.py validate \
  --manifest tests/fixtures/corpus/corpus-manifest.json
```

#### corpus-manifest.json 结构

```json
{
  "version": "corpus-manifest/1",
  "generated_at": "2026-06-27T...",
  "total_sources": 459,
  "sources": [
    {
      "id": "src-001",
      "book_source_url": "https://www.sudugu.org",
      "book_source_name": "速读谷吧",
      "book_source_type": 0,
      "rule_forms": ["css-shorthand", "xpath", "regex-suffix", "put-get", "js"],
      "has_js": true,
      "has_multirule": false,
      "has_login": false,
      "search_url_pattern": "https://www.sudugu.org/sa/all-{key}-1.html",
      "fixture_file": "legado_sudugu_vertical.json",
      "recorded": true,
      "batch_priority": "P0"
    }
  ],
  "by_form": {
    "css-shorthand": 208,
    "xpath": 4,
    "json-jsonpath": 195,
    "js": 335,
    "multirule": 294,
    "regex": 0
  },
  "by_priority": { "P0": 30, "P1": 100, "P2": 329 }
}
```

#### 分类规则

- **P0**（优先验证）: 覆盖独特规则形态组合的源，每种形态至少 5 个，共 ~30 个
- **P1**（常规验证）: 规则形态常见但不独特的源，~100 个
- **P2**（长尾）: 剩余源，~329 个

分类逻辑：按 `rule_forms` 组合去重，每种组合选 5 个代表源入 P0。

### 3.3 脱敏规则

```python
SENSITIVE_PATTERNS = [
    (r'token=[a-f0-9]{32}', 'token=<REDACTED>'),
    (r'cookie:\s*[\w=/+; -]+', 'cookie: <REDACTED>'),
    (r'apikey=[\w-]+', 'apikey=<REDACTED>'),
    (r'Bearer\s+[\w.-]+', 'Bearer <REDACTED>'),
]
```

录制响应时自动扫描并替换，生成 `sanitize-report.json` 记录脱敏了哪些字段。

---

## 4. 批量测试工具 (corpus-batch-runner)

### 4.1 目标

用 459 源集合（或 P0 抽样）批量跑 import→search→detail→toc→content，统计通过率。

### 4.2 新增 reader-cli 子命令

```bash
# 批量测试（离线回放模式，用录制的响应）
cargo run -p reader-cli -- --corpus-batch \
  --manifest tests/fixtures/corpus/corpus-manifest.json \
  --recorded-dir tests/fixtures/corpus/recorded/ \
  --out reports/tooling/corpus-batch-result.json \
  [--max-sources 30] \
  [--stop-on-failure]

# 批量测试（真实网络模式，可选）
cargo run -p reader-cli -- --corpus-batch-live \
  --manifest tests/fixtures/corpus/corpus-manifest.json \
  --keyword "斗破苍穹" \
  --out reports/tooling/corpus-batch-live-result.json \
  --timeout 15 \
  [--max-sources 10]
```

### 4.3 评估规则 — 5 级通过定义

每个源在批量测试中按 5 级评估：

| 级别 | 步骤 | 通过条件 | 证据 |
| --- | --- | --- | --- |
| **L1-import** | `source.import` | 返回 `result`（非 error），sourceId 非空 | protocol event |
| **L2-search** | `book.search` | 返回 ≥1 本书，每本 title 非空 | protocol event |
| **L3-detail** | `book.detail` | 返回 book，name 非空 | protocol event |
| **L4-toc** | `book.toc` | 返回 ≥1 章节，每章 title 非空 | protocol event |
| **L5-content** | `chapter.content` | 返回正文，content 非空且 >50 字符 | protocol event |

**一个源"完全通过" = L1-L5 全绿。**

### 4.4 结果格式

```json
{
  "version": "corpus-batch-result/1",
  "generated_at": "2026-06-27T...",
  "mode": "offline-replay",
  "total": 30,
  "summary": {
    "fully_passed": 18,
    "partially_passed": 8,
    "fully_failed": 4,
    "pass_rate": 0.60,
    "by_level": {
      "L1-import": { "passed": 28, "failed": 2 },
      "L2-search": { "passed": 22, "failed": 8 },
      "L3-detail": { "passed": 20, "failed": 10 },
      "L4-toc": { "passed": 18, "failed": 12 },
      "L5-content": { "passed": 18, "failed": 12 }
    }
  },
  "sources": [
    {
      "source_id": "src-001",
      "source_name": "速读谷吧",
      "levels": {
        "L1-import": "pass",
        "L2-search": "pass",
        "L3-detail": "pass",
        "L4-toc": "pass",
        "L5-content": "pass"
      },
      "fully_passed": true,
      "failure_detail": null
    },
    {
      "source_id": "src-005",
      "source_name": "有度轻说",
      "levels": {
        "L1-import": "pass",
        "L2-search": "fail",
        "L3-detail": "skip",
        "L4-toc": "skip",
        "L5-content": "skip"
      },
      "fully_passed": false,
      "failure_detail": "L2-search: parseFailed(bookList container produced no matches) — MultiRule blocker"
    }
  ]
}
```

### 4.5 Gate 规则

- **PR gate**: 不跑批量（太慢）
- **main gate**: 跑 P0 抽样（30 个），pass_rate ≥ 80%
- **nightly**: 跑全量 459，pass_rate ≥ 60%（初始阈值，逐步提高）
- 失败源自动生成 release blocker（`release_blocker_register.py` 从 result 生成）

---

## 5. 真实网络冒烟 (live-smoke)

### 5.1 目标

fixture-vertical 用静态录制响应，live-smoke 对真实站点发起请求，验证网络变化场景。

### 5.2 工具

```bash
# 对 P0 源发起真实搜索请求
cargo run -p reader-cli -- --live-smoke \
  --manifest tests/fixtures/corpus/corpus-manifest.json \
  --keyword "斗破苍穹" \
  --out reports/tooling/live-smoke-result.json \
  --timeout 15 \
  --max-sources 10
```

### 5.3 验证场景

- 真实 HTTP 请求（非 mock）
- 重定向跟随
- 编码检测（GBK/UTF-8）
- Cookie 传递
- JS 执行（如果源含 `@js:`，验证 JS sandbox 端到端）

### 5.4 Gate 规则

- **不在 PR/main gate 跑**（依赖外部网络，不稳定）
- **nightly 跑**：P0 抽样 10 个，pass_rate ≥ 70%
- **手动触发**：GitHub Action `workflow_dispatch`

---

## 6. 四端 Corpus Benchmark (S7)

### 6.1 目标

证明 CLI / iOS / Android / HarmonyOS 对同一 corpus 读出同样结果。

### 6.2 流程

```
1. 选定 corpus（P0 抽样 10 个源 + 录制响应）
2. 四端各跑一遍：
   - CLI: reader-cli --corpus-batch（离线回放）
   - Android: connectedAndroidTest --corpus-batch（MockWebServer 回放）
   - iOS: xctest --corpus-batch（URLProtocol mock 回放）
   - HarmonyOS: hdc test --corpus-batch（OHOS HTTP mock 回放）
3. 每端产出 platform-result.json（标准化格式）
4. cross_platform_diff.py 对比四端结果
5. 差异 → release blocker
```

### 6.3 标准化结果格式（复用 platform-evidence-validator）

```json
{
  "version": "platform-evidence/1",
  "platform": "cli",
  "kind": "corpus",
  "capability": "remote.reading.v1",
  "status": "pass",
  "timestamp": "2026-06-27T...",
  "environment": { "os": "macos", "arch": "arm64", "toolchain": "rust-1.87" },
  "fixture_id": "src-001",
  "artifact": "reports/tooling/corpus/cli/src-001.json",
  "notes": "L1-L5 all pass"
}
```

### 6.4 Diff 规则

`cross_platform_diff.py` 对比四端每个源的 L1-L5 结果：
- 全部一致（全 pass 或全 fail 且 fail 原因相同）→ `parity: true`
- 不一致 → `parity: false`，生成 diff report + release blocker

### 6.5 Gate 规则

- **release gate**: 四端 parity ≥ 90%（10 个源中 ≥9 个一致）
- **非 PR gate**（需要四端模拟器/设备，太慢）
- **手动触发 + release 前**

---

## 7. 证据索引与能力目录

### 7.1 evidence-indexer（已有工具，需接入）

```bash
# 扫描所有测试产出，生成统一证据索引
python3 tools/evidence-indexer/evidence_indexer.py . \
  --out reports/tooling/evidence-index.json
```

每条证据标注：
- `tier`: unit / conformance / fixture / batch / live / device / corpus
- `platform`: cli / ios / android / harmony / host
- `capability`: remote.reading.v1 / local-book / rss / sync / tts 等
- `status`: pass / fail / unknown

### 7.2 capability-catalog（已有工具，需接入）

```bash
# 扫描 protocol/ + host-contracts/，生成能力目录
python3 tools/capability-catalog/capability_catalog.py . \
  --out reports/tooling/capability-catalog.json
```

每个能力标注：core-owned / host-owned / shared，implemented / missing。

### 7.3 gate-declaration（已有工具，需接入）

```bash
# 验证三端 gate 是否声明
python3 tools/gate-declaration/gate_declaration.py .
```

---

## 8. 评估报告自动化

### 8.1 每轮测试产出

```
reports/tooling/
├── evidence-index.json          ← evidence-indexer 产出
├── capability-catalog.json      ← capability-catalog 产出（已有）
├── release-blockers.json        ← 手工 + release_blocker_register 自动补充
├── corpus-batch-result.json     ← corpus-batch-runner 产出
├── corpus-batch-live-result.json ← live-smoke 产出（nightly）
├── corpus-benchmark/
│   ├── cli/                     ← CLI corpus 结果
│   ├── android/                 ← Android corpus 结果
│   ├── ios/                     ← iOS corpus 结果
│   ├── harmony/                 ← HarmonyOS corpus 结果
│   └── diff-report.json         ← cross_platform_diff 产出
└── gate-declaration.json        ← gate-declaration 产出
```

### 8.2 评估看板（生成 Markdown）

```bash
python3 tools/corpus-manager/corpus_manager.py report \
  --evidence reports/tooling/evidence-index.json \
  --batch reports/tooling/corpus-batch-result.json \
  --blockers reports/tooling/release-blockers.json \
  --out reports/tooling/assessment.md
```

产出 `assessment.md` 包含：
- 能力覆盖矩阵（能力 × 证据层级 × 平台）
- 书源通过率（L1-L5 分级统计）
- blocker 汇总（blocker / medium / low）
- gate 状态（PR / main / nightly / release 各级是否通过）

---

## 9. CI 流程设计

### 9.1 PR Gate（每次 PR，~3 分钟）

```yaml
pr-gate:
  steps:
    - cargo fmt --check
    - cargo clippy --workspace -- -D warnings
    - cargo test --workspace
    - cargo run -p reader-cli -- --conformance
    - python3 tools/protocol-schema-lint/protocol_schema_lint.py
```

### 9.2 Main Gate（合并到 main，~10 分钟）

```yaml
main-gate:
  steps:
    - # PR gate 全部
    - cargo test -p reader-cli --test fixture_vertical --test host_replay
    - # P0 抽样批量测试（离线回放，30 个源）
    - cargo run -p reader-cli -- --corpus-batch
        --manifest tests/fixtures/corpus/corpus-manifest.json
        --max-sources 30
        --out reports/tooling/corpus-batch-result.json
    - # 证据索引
    - python3 tools/evidence-indexer/evidence_indexer.py . --out reports/tooling/evidence-index.json
    - # 能力目录
    - python3 tools/capability-catalog/capability_catalog.py . --out reports/tooling/capability-catalog.json
    - # Gate 声明
    - python3 tools/gate-declaration/gate_declaration.py .
    - # 评估报告
    - python3 tools/corpus-manager/corpus_manager.py report ...
```

### 9.3 Nightly（每天，~30 分钟）

```yaml
nightly:
  schedule: "0 2 * * *"
  steps:
    - # main gate 全部
    - # 全量批量测试（459 源离线回放）
    - cargo run -p reader-cli -- --corpus-batch
        --manifest tests/fixtures/corpus/corpus-manifest.json
        --out reports/tooling/corpus-batch-full.json
    - # 真实网络冒烟（P0 抽样 10 个）
    - cargo run -p reader-cli -- --live-smoke
        --manifest tests/fixtures/corpus/corpus-manifest.json
        --max-sources 10
        --out reports/tooling/live-smoke-result.json
    - # 自动更新 blocker
    - python3 tools/release-blocker-register/release_blocker_register.py
        --batch reports/tooling/corpus-batch-full.json
        --out reports/tooling/release-blockers.json
```

### 9.4 Release Gate（手动触发，~60 分钟）

```yaml
release-gate:
  workflow_dispatch:
  steps:
    - # nightly 全部
    - # 四端 corpus benchmark
    - # CLI
    - cargo run -p reader-cli -- --corpus-batch --out reports/tooling/corpus-benchmark/cli/
    - # Android（需要 macOS runner + Android SDK + emulator）
    - ./gradlew :app:connectedDebugAndroidTest --tests "*.CoreCorpusBenchmark"
    - # iOS（需要 macOS runner + Xcode + simulator）
    - xcodebuild test -scheme ReaderForIOSApp -only-testing "*.CorpusBenchmark"
    - # HarmonyOS（需要 DevEco + simulator，当前无法 CI，手动跑）
    - # Diff
    - python3 tools/cross-platform-diff/cross_platform_diff.py
        --cli reports/tooling/corpus-benchmark/cli/
        --android reports/tooling/corpus-benchmark/android/
        --ios reports/tooling/corpus-benchmark/ios/
        --harmony reports/tooling/corpus-benchmark/harmony/
        --out reports/tooling/corpus-benchmark/diff-report.json
    - # Parity gate
    - python3 tools/corpus-manager/corpus_manager.py parity-gate
        --diff reports/tooling/corpus-benchmark/diff-report.json
        --threshold 0.90
```

---

## 10. 实施优先级

### Phase 1: 书源管理 + 批量测试基础（P0，1-2 天）

1. **新增 `corpus_manager.py`** — import + classify + sanitize
2. **导入 459 源集合** → `tests/fixtures/corpus/sources/`
3. **生成 corpus-manifest.json** — 分类索引
4. **新增 reader-cli `--corpus-batch` 子命令** — 离线回放批量测试
5. **跑 P0 抽样 30 个** — 生成第一份 corpus-batch-result.json
6. **定义 5 级评估规则** — L1-L5 通过定义

### Phase 2: 证据自动化 + CI 接入（P1，1 天）

7. **接入 evidence-indexer** — 扫描所有测试产出
8. **接入 capability-catalog** — 生成能力目录
9. **接入 gate-declaration** — 验证三端 gate 声明
10. **扩展 CI** — main gate 加 P0 批量 + 证据索引
11. **生成评估看板** — assessment.md

### Phase 3: 真实网络 + 录制（P1，1-2 天）

12. **新增 `corpus_manager.py record`** — 录制真实响应
13. **新增 reader-cli `--live-smoke`** — 真实网络冒烟
14. **Nightly CI** — 全量批量 + live-smoke
15. **自动 blocker 生成** — release_blocker_register 从结果生成

### Phase 4: 四端 Corpus Benchmark（P2，S7 阶段）

16. **Android corpus benchmark test** — MockWebServer 回放
17. **iOS corpus benchmark test** — URLProtocol mock 回放
18. **HarmonyOS corpus benchmark test** — OHOS HTTP mock 回放
19. **cross_platform_diff** — 四端结果对比
20. **Release gate CI** — 手动触发四端 benchmark

---

## 11. "跑通"的精确定义（评估规则）

### 11.1 单源通过定义

一个真实 Legado 书源"跑通" = 以下 5 级全部 pass：

| 级别 | 命令 | 通过条件 | 失败处理 |
| --- | --- | --- | --- |
| L1 | `source.import` | result + sourceId 非空 | 记 blocker(import) |
| L2 | `book.search` | ≥1 结果 + title 非空 | 记 blocker(search) |
| L3 | `book.detail` | name 非空 | 记 blocker(detail) |
| L4 | `book.toc` | ≥1 章节 + title 非空 | 记 blocker(toc) |
| L5 | `chapter.content` | content >50 字符 | 记 blocker(content) |

### 11.2 能力达标定义

| 能力 | 达标条件 |
| --- | --- |
| 书源解析(CSS) | P0 中 CSS 源 ≥80% L1-L5 全通 |
| 书源解析(XPath) | P0 中 XPath 源 ≥80% L1-L5 全通 |
| 书源解析(JSON) | P0 中 JSON 源 ≥80% L1-L5 全通 |
| 书源解析(JS) | P0 中 JS 源 ≥80% L1-L5 全通 |
| 书源解析(MultiRule) | P0 中 MultiRule 源 ≥80% L1-L5 全通 |
| 本地书(EPUB/TXT) | crate test + 真实文件验证 |
| RSS | 真实 RSS 源解析 ≥1 个 |
| 同步 | 真实 WebDAV 服务器往返 ≥1 次 |
| TTS | 队列状态机 conformance + Host 发声验证 |

### 11.3 Release Gate 定义

| Gate | 通过条件 |
| --- | --- |
| PR | fmt + clippy + workspace test + conformance |
| Main | PR + fixture_vertical + P0 批量 ≥80% + evidence-index |
| Nightly | Main + 全量批量 ≥60% + live-smoke ≥70% + blocker 自动更新 |
| Release | Nightly + 四端 corpus parity ≥90% + blocker=0 |

---

## 12. 与现有工具的关系

| 现有工具 | 处置 |
| --- | --- |
| `conformance.rs` | 保留，PR gate |
| `fixture_vertical.rs` | 保留，main gate |
| `host_replay.rs` | 保留，main gate |
| `fixture_manifest.py` | 保留，被 corpus_manager 调用 |
| `corpus_batch_selector.py` | 保留，被 corpus_manager 调用（P0/P1/P2 分批） |
| `corpus_real_run_collector.py` | 保留，S7 四端收集 |
| `cross_platform_diff.py` | 保留，S7 diff |
| `evidence_indexer.py` | **接入**，main gate 产出 evidence-index.json |
| `capability_catalog.py` | **接入**，main gate 产出 capability-catalog.json |
| `gate_declaration.py` | **接入**，main gate 验证三端 gate 声明 |
| `platform_evidence_validator.py` | 保留，S7 四端结果校验 |
| `release_blocker_register.py` | **接入**，nightly 从批量结果自动生成 blocker |
| `benchmark_run_packager.py` | 保留，S7 打包 |
| `protocol_schema_lint.py` | **接入**，PR gate |
| `corpus_booksource_oracle.py` | 保留，被 corpus_manager 调用 |
| `corpus_canonicalize.py` | 保留，S7 canonical |

新增工具：
- `corpus_manager.py` — 书源管理核心
- reader-cli `--corpus-batch` — 批量测试子命令
- reader-cli `--live-smoke` — 真实网络冒烟子命令

---

*本文为测试验证体系规划文档，实施按 Phase 1-4 顺序推进。*
