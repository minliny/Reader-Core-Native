# Legado 能力缺口补齐方案

日期：2026-06-27
前置文档：`docs/LEGADO_CAPABILITY_INVENTORY.md`（97 项能力清单）

本文是对 45 项未实现 + 16 项部分实现能力的补齐方案，按优先级和依赖关系排序。
每项标注 Legado 源码路径、缺口类型、实现方案。

---

## 优先级定义

- **P0-blocker**：阻断真实书源跑通，不补则 459 源批量测试大面积失败
- **P1-core**：Legado 核心能力，影响大量源或基础体验
- **P2-important**：重要能力，影响部分源或进阶体验
- **P3-deferred**：可延后，Host/UI 层或低频能力

---

## P0-blocker：阻断真实书源的能力

### 1. MultiRule 操作符拆分（&&/||/%%）

| 项 | 值 |
|----|-----|
| 缺口 | CSS 路径不拆分 &&/\|\|/%% |
| Legado 源码 | AnalyzeRule.kt:485 `splitSourceRule()` |
| 影响 | 459 源中 292 源含 MultiRule（64%），不补则大面积 L2-search 失败 |
| 方案 | reader-rule 中 `splitSourceRule` 实现：按 &&/\|\|/%% 拆分为 SourceRule 列表，每条独立解析后合并结果 |
| 验收 | 含 MultiRule 的 P0 源 L2-search 通过率 ≥80% |
| 状态 | 已有 release blocker `rb-legado-css-multirule-operator`。Agent 正在修，
         reader-rule +391 行未提交，15 multirule tests pass (&&/||/%% CSS+XPath 拆分)。
         等待 agent 完成后提交 + 跑 459 源批量验证 |

### 2. 多页加载（nextTocUrl / nextContentUrl）

| 项 | 值 |
|----|-----|
| 缺口 | 目录和正文不支持翻页加载 |
| Legado 源码 | BookChapterList.kt:192 `nextTocUrl`、BookContent.kt:185 `nextContentUrl` |
| 影响 | 大量源目录分页、正文分页，不补则 L4-toc / L5-content 只能取第一页 |
| 方案 | reader-runtime/remote.rs 中 `book_toc` / `chapter_content` 完成后检查 nextUrl，循环发 http.execute 直到无下一页，合并结果 |
| 验收 | 有 nextTocUrl 的源能取完整目录；有 nextContentUrl 的源能取完整正文 |

### 3. 规则补全（RuleComplete）

| 项 | 值 |
|----|-----|
| 缺口 | 简单规则不自动补全 @text/@href/@src |
| Legado 源码 | RuleComplete.kt `autoComplete()` |
| 影响 | 大量源规则省略尾部操作符（如 `div.class&&` 后无 @text），不补则解析返回空 |
| 方案 | reader-rule 中实现 `RuleComplete::auto_complete()`：正则识别缺尾操作符的规则，按 type(文字/链接/图片) 补全 |
| 验收 | 省略尾操作符的源能正确解析 |

---

## P1-core：Legado 核心能力

### 4. 替换规则（ReplaceRule）

| 项 | 值 |
|----|-----|
| 缺口 | 正文替换规则完全未实现 |
| Legado 源码 | ReplaceRule.kt（实体）+ ContentProcessor.kt:91 `getContent()` + ReplaceAnalyzer.kt |
| 影响 | Legado 核心内容处理能力，用户自定义净化/替换无替代 |
| 方案 | 1) reader-domain 新增 ReplaceRule 实体（pattern/replacement/scope/scopeTitle/scopeContent/isRegex/order）
2) reader-content 新增 ContentProcessor：按 scope 匹配 → regex 替换 → 按排序执行
3) protocol 新增 replace-rule CRUD 命令
4) chapter.content 返回前过 ContentProcessor |
| 验收 | 导入 Legado 替换规则 JSON → 对正文执行替换 → 结果正确 |

### 5. 繁简转换（ChineseConverter）

| 项 | 值 |
|----|-----|
| 缺口 | t2s/s2t 繁简转换未实现 |
| Legado 源码 | ChineseUtils.kt（封装 quick-transfer 库）+ JsExtensions.kt:547 `t2s()`/`s2t()` |
| 影响 | JsExtensions 中 JS 可调用 t2s/s2t；阅读引擎按配置自动转换 |
| 方案 | 1) 引入 Rust 繁简转换 crate（如 `chinese_converter` 或内嵌字典）
2) reader-js 暴露 java.t2s()/java.s2t()
3) ContentProcessor 按配置执行转换 |
| 验收 | JS 中 `java.t2s("測試")` 返回 `"测试"`；正文按配置自动转换 |

### 6. TXT 目录规则（TxtTocRule）

| 项 | 值 |
|----|-----|
| 缺口 | TXT 本地书目录识别规则未实现 |
| Legado 源码 | TxtTocRule.kt（实体）+ LocalBook.kt 中应用 |
| 影响 | TXT 本地书无法自动分章 |
| 方案 | 1) reader-domain 新增 TxtTocRule 实体（name/rule/example/serialNumber/enable）
2) reader-local-book/txt.rs 中按 TxtTocRule 正则分章
3) protocol 新增 txt-toc-rule CRUD 命令 |
| 验收 | 导入 Legado TXT 目录规则 → 对真实 TXT 文件分章 → 结果与 Legado 一致 |

### 7. 书签（Bookmark）

| 项 | 值 |
|----|-----|
| 缺口 | 书签实体和持久化未实现 |
| Legado 源码 | Bookmark.kt（time/bookName/bookAuthor/chapterIndex/chapterPos/bookText） |
| 影响 | 阅读体验基础功能 |
| 方案 | 1) reader-domain 新增 Bookmark 实体
2) reader-storage SQLite 表
3) protocol 新增 bookmark CRUD 命令 |
| 验收 | 创建/删除/查询书签 → 持久化 → 重启后恢复 |

### 8. 书架分组（BookGroup）

| 项 | 值 |
|----|-----|
| 缺口 | 书架分组未实现 |
| Legado 源码 | BookGroup.kt（groupId/groupName/cover/order/enableRefresh/show） |
| 影响 | 书架管理基础 |
| 方案 | 1) reader-domain 新增 BookGroup 实体
2) reader-storage SQLite 表
3) protocol 新增 book-group CRUD + bookshelf.list 支持按分组过滤 |
| 验收 | 创建分组 → 分配书到分组 → 按分组查询书架 |

### 9. 阅读记录（ReadRecord）

| 项 | 值 |
|----|-----|
| 缺口 | 阅读时长记录未实现 |
| Legado 源码 | ReadRecord.kt（deviceId/bookName/readTime/lastRead） |
| 影响 | 阅读统计 |
| 方案 | 1) reader-domain 新增 ReadRecord 实体
2) reader-storage SQLite 表
3) reading.progress.update 时累计 readTime |
| 验收 | 阅读 → 累计时长 → 查询统计 |

### 10. 发现（Explore）

| 项 | 值 |
|----|-----|
| 缺口 | 发现页 exploreUrl 解析和协议方法未实现 |
| Legado 源码 | WebBook.kt:93 `exploreBook()` + BookSourceExtensions.kt:44 `getExploreKinds()` |
| 影响 | 书源发现页功能不可用 |
| 方案 | 1) reader-content 实现 exploreUrl 解析：`名称::url\n名称::url` 或 JSON 数组格式
2) reader-runtime 新增 `source.explore` 协议方法
3) 复用 BookList 解析逻辑（explore 和 search 共用 BookListRule） |
| 验收 | 有 exploreUrl 的源能列出发现分类 → 选分类 → 返回书列表 |

---

## P2-important：重要能力

### 11. 段评（ReviewRule）

| 项 | 值 |
|----|-----|
| 缺口 | 书评/段评规则未实现 |
| Legado 源码 | ReviewRule.kt（reviewUrl/avatarRule/contentRule/postTimeRule）+ BookChapterReview.kt |
| 方案 | 1) reader-domain 新增 ReviewRule 字段（已在 BookSource 中预留 ruleReview）
2) protocol 新增 `review.list` 命令
3) reader-runtime 实现段评获取（复用 AnalyzeRule 解析） |
| 验收 | 有 ruleReview 的源能获取段评列表 |

### 12. 字体反混淆（QueryTTF）

| 项 | 值 |
|----|-----|
| 缺口 | QueryTTF 字体反混淆未实现 |
| Legado 源码 | QueryTTF.java（解析 TTF 字体文件，映射字形到字符） |
| 影响 | 反爬能力，部分源用自定义字体混淆正文 |
| 方案 | 1) reader-rule 或 reader-content 实现 TTF 解析（读取 cmap 表映射）
2) reader-js 暴露 `java.queryTTF()` |
| 验收 | 含字体混淆的源能还原正确文字 |

### 13. 封面解密（coverDecodeJs）

| 项 | 值 |
|----|-----|
| 缺口 | 封面图片解密 JS 未实现 |
| Legado 源码 | ImageUtils.kt:66 + BookSource.kt:69 `coverDecodeJs` |
| 影响 | 部分源封面加密 |
| 方案 | reader-js 暴露 coverDecodeJs 执行 → Host 用返回的 bytes 加载图片 |
| 验收 | 含 coverDecodeJs 的源封面正确显示 |

### 14. 全文搜索（SearchContent）

| 项 | 值 |
|----|-----|
| 缺口 | 书内全文搜索未实现 |
| Legado 源码 | SearchContentViewModel.kt（遍历章节正文匹配关键词） |
| 影响 | 阅读体验 |
| 方案 | 1) protocol 新增 `book.searchContent` 命令
2) reader-storage 或 reader-runtime 遍历已缓存章节正文匹配
3) 返回 SearchResult（chapterIndex/position/snippet） |
| 验收 | 输入关键词 → 返回所有匹配章节和位置 |

### 15. 换源

| 项 | 值 |
|----|-----|
| 缺口 | 换源功能未实现 |
| Legado 源码 | ChangeBookSourceDialog.kt + ChangeChapterSourceDialog.kt |
| 影响 | 阅读体验，同一书切换不同源 |
| 方案 | 1) 用 book.search 搜索同名书 → 列出可用源
2) protocol 新增 `book.changeSource` 命令
3) 保留阅读进度，切换 sourceId 重新加载目录和正文 |
| 验收 | 换源后阅读进度保持，目录和正文从新源加载 |

### 16. 内容去重标题

| 项 | 值 |
|----|-----|
| 缺口 | 去除正文开头重复书名/章节标题未实现 |
| Legado 源码 | ContentProcessor.kt:73 `upRemoveSameTitle()` |
| 方案 | reader-content 中 ContentProcessor 实现：正则匹配正文开头 `^(书名)*(章节标题)(\s)*` 删除 |
| 验收 | 正文开头重复标题被正确去除 |

### 17. 智能分段（reSegment）

| 项 | 值 |
|----|-----|
| 缺口 | 正文重新分段未实现 |
| Legado 源码 | ContentHelp.kt `reSegment()` |
| 方案 | reader-content 实现：按标点/换行/缩进规则重新分段 |
| 验收 | 无分段的正文被正确分段 |

---

## P2-数据实体（批量补齐）

以下实体只需定义结构 + SQLite 表 + CRUD 协议，无复杂逻辑：

| # | 实体 | Legado 源码 | 字段 |
|---|------|------------|------|
| 18 | DictRule | DictRule.kt | name/urlRule/showRule/enabled/sortNumber |
| 19 | HttpTTS | HttpTTS.kt | name/url/contentType/concurrentRate/loginUrl/header/jsLib |
| 20 | RuleSub | RuleSub.kt | name/url/type/customOrder/autoUpdate |
| 21 | SearchBook | SearchBook.kt | 搜索结果缓存 |
| 22 | SearchKeyword | SearchKeyword.kt | 搜索历史 |

---

## P3-deferred：可延后

| # | 能力 | 原因 |
|---|------|------|
| 23 | Umd 格式 | 低频格式 |
| 24 | RSS 收藏 (RssStar) | 低频 |
| 25 | RSS 阅读记录 (RssReadRecord) | 低频 |
| 26 | HttpTTS 在线语音 | 已有 TTS 策略文档，下一阶段 |
| 27 | Web 服务 (HttpServer) | Core 不开 socket（红线 4） |
| 28 | ContentProvider API | 平台层 |
| 29 | 后台服务 (7 个 Service) | Host 层 |
| 30 | 漫画阅读 (ReadManga) | UI 层 |
| 31 | 听书 (AudioPlay) | UI 层 |
| 32 | 书源校验 (CheckSource) | 可用测试工具链替代 |
| 33 | 书源调试 (Debug WebSocket) | 可用 CLI 替代 |
| 34 | 规则订阅 (RuleSub autoUpdate) | 低频 |
| 35 | 文件上传 (multipart) | 低频 |
| 36 | 旧数据迁移 | 不需要 |
| 37 | GlideUrl 图片加载 | Host 层 |
| 38 | 后台下载/缓存导出 | Host 层 |

---

## 部分实现能力的验证补齐

以下 16 项已有实现但从未用真实 Legado 数据验证，需用测试工具链批量验证：

| # | 能力 | 验证方式 |
|---|------|---------|
| 1 | CSS 选择器 | 459 源 L2-L5 批量测试 |
| 2 | XPath | 459 源中 3 个 XPath 源验证 |
| 3 | JSONPath | 459 源中 195 个 JSON 源验证 |
| 4 | 正则 (regex-suffix) | 459 源中 204 个含 ## 的源验证 |
| 5 | @put/@get | 459 源中 66 个含 @put/@get 的源验证 |
| 6 | {{}} 模板 | 459 源中 225 个含模板的源验证 |
| 7 | <js> 内联 JS | 459 源中 157 个含内联 JS 的源验证 |
| 8 | @js: 规则 | 459 源中 158 个含 @js: 的源验证 |
| 9 | JS URL 构造 | 459 源中 59 个 @js: searchUrl 验证 |
| 10 | charset 编码 | 459 源中含 GBK 的源验证 |
| 11 | Cookie 传递 | 459 源中含 Cookie 的源验证 |
| 12 | Mobi 格式 | 真实 Mobi 文件验证 |
| 13 | RSS 默认解析 | 真实 RSS 源验证 |
| 14 | RSS 文章内容 | 真实 RSS 源验证 |
| 15 | WebDAV 同步 | 真实 WebDAV 服务器往返 |
| 16 | 备份/恢复 AES | 真实备份文件验证 |

---

## 实施顺序

```
Phase A (P0-blocker, 阻断测试):
  1. MultiRule 拆分 ← Agent 正在修
  2. 多页加载 (nextTocUrl/nextContentUrl)
  3. 规则补全 (RuleComplete)

Phase B (P1-core, 基础能力):
  4. 替换规则 (ReplaceRule)
  5. 繁简转换
  6. TXT 目录规则
  7-9. 书签/书架分组/阅读记录 (数据实体批量)
  10. 发现 (Explore)

Phase C (P2, 进阶能力):
  11-17. 段评/字体反混淆/封面解密/全文搜索/换源/去重标题/智能分段
  18-22. 数据实体批量补齐

Phase D (验证):
  16 项部分实现能力用测试工具链批量验证

Phase E (P3, 延后):
  23-38. 低频/Host层能力
```

---

## 与测试工具链的配合

1. **测试工具链先建好**（当前正在开发），产出 459 源 L1-L5 真实通过率
2. **通过率暴露真实缺口**：哪些源因为 MultiRule 失败、哪些因为多页失败、哪些因为规则补全失败
3. **按失败原因排序补齐**：失败最多的缺口优先补
4. **每补一个能力 → 重跑批量测试** → 通过率应有可见提升
5. **通过率不提升的** → 可能是实现有 bug，需调试

**核心原则：通过率驱动开发优先级，不凭猜测排序。**

---

*本文为能力缺口补齐方案，与 `docs/LEGADO_CAPABILITY_INVENTORY.md` 配合使用。
任何能力补齐后必须更新清单状态并用测试工具链验证。*
