# Corpus 审计报告：第 1 轮

> 当前最高优先级文档是 `docs/LOCAL_REPO_MIGRATION_DIRECTIVE.md`。本文是历史 corpus
> seed 记录，后续 benchmark 必须围绕本地旧 Core 到 Rust Core、再到三端一致性更新。

- **分支：** `codex/goal-sanitized-corpus`
- **基线：** `origin/codex/core-product-integration` at `fb4c3a7`
- **轮次：** 1，初始 batch
- **日期：** 2026-06-25
- **范围：** 建立 sanitized corpus 目录结构，并为每种已规划 source type 放入一个
  seed fixture。本分支不修改代码。

## 1. 本轮新增 corpus item

| ID | 类型 | 格式 | Fixture | Manifest | 主要消费分支 |
| --- | --- | --- | --- | --- | --- |
| `bs-001` | book-source | json | `fixtures/sanitized-corpus/book-source/bs-001-fixture.json` | `bs-001.manifest.json` | `codex/rule-engine-parity` |
| `wp-001` | web-page | html | `fixtures/sanitized-corpus/web-page/wp-001-fixture.html` | `wp-001.manifest.json` | `codex/rule-engine-parity` |
| `ja-001` | json-api | json | `fixtures/sanitized-corpus/json-api/ja-001-fixture.json` | `ja-001.manifest.json` | `codex/rule-engine-parity` |
| `xf-001` | xml-feed | xml | `fixtures/sanitized-corpus/xml-feed/xf-001-fixture.xml` | `xf-001.manifest.json` | `codex/rule-engine-parity` |
| `rf-001` | rss-feed | xml | `fixtures/sanitized-corpus/rss-feed/rf-001-fixture.xml` | `rf-001.manifest.json` | `codex/remote-reading-vertical` |

每个 manifest 记录 `id`、`source_type`、`source_description`、`sanitization`、
`capability_tags`、`privacy_check`、`consumer_branch`、`fixture_file`、`format`、
`added_in_round`。

## 2. 能力覆盖

- Rule engine：`bs-001`、`ja-001` 覆盖 JSONPath；`wp-001` 覆盖 CSS selector、
  HTML entity、missing href；`xf-001` 覆盖 XML/namespace-aware parsing；
  `bs-001` 覆盖 book-source 的 search/detail/toc/content rule set。
- Remote reading：`rf-001` 覆盖 RSS feed iteration 和字段抽取；`bs-001` 覆盖
  book-source end-to-end sample；`ja-001` 覆盖 JSON search API。
- 本地内容解析：`wp-001` 覆盖 static HTML listing；`xf-001` 覆盖
  Atom/OPDS catalog。

`ja-001` 特意保留可扩展结构，用于后续 JSONPath filter
`[?(@.meta.rating>4.0)]`、slice、recursive-descent、union 表达式测试。

## 3. 隐私验证

五个 item 均通过隐私检查。检查项包括：

| 检查项 | `bs-001` | `wp-001` | `ja-001` | `xf-001` | `rf-001` |
| --- | :---: | :---: | :---: | :---: | :---: |
| 真实 token / API key | pass | pass | pass | pass | pass |
| Cookie / auth header | pass | pass | pass | pass | pass |
| 账号凭据 | pass | pass | pass | pass | pass |
| 私有内容 | pass | pass | pass | pass | pass |
| 长版权文本 | pass | pass | pass | pass | pass |
| HTML tracking script | n/a | pass | n/a | n/a | n/a |

结论：fixture 中不包含真实 token、cookie、账号、私有正文或长版权文本。hostname 使用
`example.test`、`img.example.test`、`feed.example.test` 或相对 URL。标题、作者和正文
均为合成占位内容。

## 4. 路径和范围合规

本轮只触碰允许路径：

- `fixtures/sanitized-corpus/**`：10 个新文件，5 个 fixture 和 5 个 manifest
- `reports/corpus-audit/**`：1 个审计报告

确认未修改：

- `tests/**`
- `crates/**`
- `protocol/**`
- `bindings/**`
- `scripts/**`
- `tools/**`
- `Cargo.*`

本分支没有接入代码；corpus 只是数据，供后续长期分支消费。

## 5. 目录约定

```text
fixtures/sanitized-corpus/
  <source-type>/
    <id>-fixture.<ext>
    <id>.manifest.json
reports/corpus-audit/
  <NNNN>-<slug>.md
```

ID 在 source type 内补零，例如 `bs-001`、`wp-001`。报告文件轮次也补零，例如
`0001-...`。

## 6. 后续轮次

- 增加覆盖 `@text` / `@html` pseudo-attribute selector 与 `:contains(...)` 的
  book-source fixture。
- 增加覆盖 JSONPath filter、union、slice 的 JSON fixture。
- 增加覆盖 pagination 与 next-link 的 multi-page HTML fixture。
- 增加覆盖 download/cover extraction 的 sanitized RSS-with-enclosure fixture。
- 后续每轮保持一个新 group、一个审计报告、一个 commit。
