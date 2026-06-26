# Corpus 审计报告：第 2 轮 — Reader-Core 迁移 batch

> 闭环 P2 — S4 远程阅读 e2e 收尾的第三项任务：迁移更多旧 Reader-Core sample 为
> sanitized Rust fixture。本批从 `Documents/Reader-Core` 选取 7 个高价值样本，
> 重消毒后落入 `fixtures/sanitized-corpus/**`。

- **分支：** `codex/p2-s4-remote-reading-e2e-closure`
- **基线：** `origin/main` at `2ca3454`
- **轮次：** 2，Reader-Core 迁移 batch
- **日期：** 2026-06-27
- **范围：** 从旧 Reader-Core 仓库迁移 7 个 fixture（4 book-source + 2 json-api
  + 1 web-page），其中 2 个需要重度消毒（case_022 真实小说站、书源示例.json 真实
  凭据）。本批只修改 `fixtures/sanitized-corpus/**`、`tests/tooling/**`、
  `reports/corpus-audit/**`。

## 1. 本轮新增 corpus item

| ID | 类型 | 格式 | Fixture | Manifest | 迁移自 | 消毒等级 |
| --- | --- | --- | --- | --- | --- | --- |
| `bs-002` | book-source | json | `bs-002-fixture.json` | `bs-002.manifest.json` | `samples/fixtures/html/sample_001_{search,toc,content}.html` | clean |
| `bs-005` | book-source | json | `bs-005-fixture.json` | `bs-005.manifest.json` | `samples/fixtures/toc/basic.{html,rule.json,expected.json}` | clean |
| `bs-009` | book-source | json | `bs-009-fixture.json` | `bs-009.manifest.json` | `case_022/{toc,detail,content}.html` | heavy |
| `bs-010` | book-source | json | `bs-010-fixture.json` | `bs-010.manifest.json` | `docs/design/书源示例.json` (393KB) | heavy |
| `ja-002` | json-api | json | `ja-002-fixture.json` | `ja-002.manifest.json` | `samples/fixtures/json/{policy_http_404,error_http_404}.json` | clean |
| `ja-003` | json-api | json | `ja-003-fixture.json` | `ja-003.manifest.json` | `samples/fixtures/group_consistency/gc_single_key_*.json` (6→3) | clean |
| `wp-002` | web-page | html | `wp-002-fixture.html` | `wp-002.manifest.json` | `samples/fixtures/html/sample_js_runtime_001_input.html` | clean |

每个 manifest 记录 `id`、`source_type`、`format`、`source_description`、
`sanitization`、`capability_tags`、`privacy_check`、`consumer_branch`、
`fixture_file`、`added_in_round=2`、`migrated_from`。

## 2. 能力覆盖增量

相对第 1 轮的 5 个 fixture，本轮新增能力覆盖：

| 能力 | 新增覆盖 fixture |
| --- | --- |
| CSS list-item text-split on pipe delimiter | `bs-002` |
| CSS attribute extraction `\|attr:href` + VIP 检测 + 分页 | `bs-005` |
| golden expected output（差分测试） | `bs-005` |
| 真实抓站 DOM 骨架（itemtxt/des/dir/list/con/prenext） | `bs-009` |
| 重复章节条目 parity（第37章 出现两次） | `bs-009` |
| POST-body JSON 模板（`{{page}}`/`{{key}}`/`{{$.id}}`） | `bs-010` |
| `@put:{key:value}` 后处理链 | `bs-010` |
| `@js:result.replace(...)` 后处理 | `bs-010` |
| `java.md5Encode(...)` sign 流 | `bs-010` |
| `java.timeFormat(...)` 时间格式化 | `bs-010` |
| JSONPath `\|\|$.data` fallback | `bs-010` |
| `##` regex 替换 | `bs-010` |
| HTTP 404 policy / transport 错误标记 | `ja-002` |
| group-key 一致性规则（pass/fail/reject） | `ja-003` |
| JS-runtime 超时 fallback 契约 | `wp-002` |

`bs-010` 对 `codex/rule-engine-parity` 分支价值最高——它完整保留了 legado
规则 DSL 的 7 类结构模式（POST-body 模板、@put 链、@js 后处理、md5 sign、
timeFormat、JSONPath fallback、regex 替换），是 rule engine parity 测试的
核心 fixture。

## 3. 重消毒记录

### bs-009（case_022 — 速读谷 / 捞尸人）

原始来源：真实抓取自 `sudugu.org`（速读谷）的小说站，包含《捞尸人》（作者
纯洁滴小龙，562.4 万字，版权小说）的完整正文、真实封面图、真实章节列表
（608 条）。

消毒操作（10 项）：

1. 真实域名 `sudugu.org` → `books.example.test` / `img.example.test`
2. 真实小说名 `捞尸人` → `示例小说`
3. 真实作者 `纯洁滴小龙` → `示例作者`
4. 真实站点名 `速读谷` → `示例站点`
5. 版权正文（content.html ~5600 字）→ 4 段合成 CJK 占位文本
6. 真实封面 URL（`sudugu.org/files/cover/...`）→ `img.example.test/cover/sample.png`
7. 章节列表从 608 条截断到 10 条（保留第 37 章重复模式用于 parity）
8. 剥离所有 `<script>` 标签、外部 CSS、tracking pixel（`tj.js`、`v7.js`）
9. 剥离相关小说侧栏（6 个真实书封 + 标题）
10. 保留 DOM 结构（class: `item`/`itemtxt`/`des`/`dir`/`list`/`con`/`prenext`/`submenu`/`header`/`menu`/`footer`）供 CSS 规则验证

### bs-010（书源示例.json — 番薯/酷我/猫眼）

原始来源：393KB JSON，3 个真实 legado 书源定义，内嵌 `api_key`、`api_secret`、
`device_id`、`uid`、md5 sign salt、真实设备指纹。

消毒操作（10 项）：

1. 所有 `api_key` → `REDACTED`（原 `20002007`）
2. 所有 `api_secret` → `REDACTED`（原 `974685bdc9957e8c`）
3. 所有 `device_id` → `00000000-0000-0000-0000-000000000000`（原真实 UUID）
4. 所有 `uid` → `00000000`（原 `60562717`）
5. 所有 md5 sign salt → `REDACTED_SALT`（原 `$%@*!^#!@(@`）
6. `g21.manmeng168.com` → `api.example.test`
7. `appi.kuwo.cn` → `api-b.example.test`
8. 源名 → `Sanitized API Source A/B`（原 番薯小说 / 酷我小说）
9. User-Agent 设备指纹 → `Sample Build`（原 `LND-AL40 Build/HONORLND-AL40`）
10. 第 3 个源（猫眼看书）作为冗余丢弃；保留 2 个结构最丰富的源

规则 DSL 结构原样保留（POST-body 模板、@put 链、@js 后处理、md5 sign、
timeFormat、JSONPath fallback、regex 替换）——这是 rule-engine-parity 的
核心测试目标。

## 4. 隐私验证

七个 item 均通过隐私检查。检查项包括：

| 检查项 | `bs-002` | `bs-005` | `bs-009` | `bs-010` | `ja-002` | `ja-003` | `wp-002` |
| --- | :---: | :---: | :---: | :---: | :---: | :---: | :---: |
| 真实 token / API key | pass | pass | pass | pass | pass | pass | pass |
| Cookie / auth header | pass | pass | pass | pass | pass | pass | pass |
| 账号凭据 | pass | pass | pass | pass | pass | pass | pass |
| 私有内容 | pass | pass | pass | pass | pass | pass | pass |
| 长版权文本 | pass | pass | pass | pass | pass | pass | pass |
| 真实域名 | pass | pass | pass | pass | pass | pass | pass |
| API secret / device_id / uid | n/a | n/a | n/a | pass | n/a | n/a | n/a |
| 设备指纹 | n/a | n/a | n/a | pass | n/a | n/a | n/a |
| tracking script | n/a | n/a | pass | n/a | n/a | n/a | pass |
| 真实封面图 | n/a | n/a | pass | n/a | n/a | n/a | n/a |

结论：consumable payload 中不包含真实 token、cookie、账号、私有正文、长版权
文本、真实域名或真实凭据。`sanitization_notes` 审计元数据允许引用原始值
（这是其用途），但被 `_consumable_payload()` 从隐私检查中剥离。

## 5. 路径和范围合规

本轮只触碰允许路径：

- `fixtures/sanitized-corpus/**`：14 个新文件（7 fixture + 7 manifest）
- `tests/tooling/**`：1 个新测试文件 `test_corpus_migration_batch.py`（31 tests）
- `reports/corpus-audit/**`：1 个审计报告

确认未修改：

- `crates/**`
- `protocol/**`
- `bindings/**`
- `scripts/**`
- `tools/**`（本轮不触碰工具代码）
- `Cargo.*`

## 6. 测试

`tests/tooling/test_corpus_migration_batch.py` 共 31 个测试，覆盖：

- `ManifestStructureTests`（7 tests）：manifest 存在、字段完整、round=2、
  fixture_file 匹配、source_type 匹配目录、capability_tags 非空、migrated_from 字段
- `PrivacyGuardTests`（4 tests）：consumable payload 无禁用字符串、
  sanitization_notes 允许引用原始值、所有 host 用 example.test、
  privacy_check.passed=true
- `Bs005GoldenExpectedTests`（2 tests）：3 条目 + VIP 标志 + URL 域名
- `Bs009SanitizationTests`（3 tests）：DOM class 保留、正文为占位文本、
  sanitization_notes 完整
- `Bs010RedactionTests`（7 tests）：2 源、api_secret REDACTED、device_id 零 UUID、
  uid 零、源名消毒、规则 DSL 模式保留、sanitization_notes 完整
- `Ja002ErrorMarkerTests`（2 tests）：2 case + 正确 marker + expected_behavior
- `Ja003GroupConsistencyTests`（4 tests）：3 case 覆盖 pass/fail/reject、
  pass 唯一、fail 重复、reject 未知规则类型
- `Wp002HtmlTests`（2 tests）：article + data-sample、无外部资源

全部 514 个 tooling 测试通过（473 旧 + 31 新 + 10 error-path）。

## 7. 后续轮次

- 增加 `@text` / `@html` pseudo-attribute selector 与 `:contains(...)` 的
  book-source fixture（第 1 轮遗留）。
- 增加 JSONPath filter `[?(@.meta.rating>4.0)]` 的 JSON fixture（第 1 轮遗留）。
- 增加 multi-page HTML pagination fixture（bs-005 已覆盖单页分页链接，
  但未覆盖跨页抓取）。
- 考虑将 `bookSourceComment` 中的 "sanitized-corpus: ..." 注释抽取为独立
  `provenance` 字段，避免与 consumable payload 混杂。
