# Legado 能力清单与 Reader 对标审计

日期：2026-06-27
审计来源：`/Users/minliny/Documents/legado` 源码（815 个 .kt/.java 文件）

本文是 Legado 全部能力的枚举清单，作为 Reader "能力底线 = Legado" 的验收基准。
任何 "能力已完成" 的判断必须对照本清单逐项验证，不得用代码量/测试数自证。

---

## 1. 书源解析引擎（model/analyzeRule + model/webBook）

### 1.1 规则语言（AnalyzeRule.kt, 84 个方法）

| 能力 | Legado 实现 | Reader Core 状态 | 证据 |
|------|------------|-----------------|------|
| CSS 选择器 (@css:/@text/@href/&&/;/\|\|) | AnalyzeByJSoup | ⚠️ 部分（MultiRule blocker 未闭环） | fixture_vertical 3 源 |
| XPath (@xpath:/@@) | AnalyzeByXPath | ✅ 有实现 | conformance |
| JSONPath ($./$[) | AnalyzeByJSonPath | ✅ 有实现 | conformance |
| 正则 (regex) | AnalyzeByRegex | ⚠️ 有 regex-suffix 但未完整验证 | 0 真实源 |
| @put/@get 变量机制 | splitPutRule/evalPattern | ✅ 已实现 | reader-rule 单元测试 |
| {{}} 模板 | evalPattern | ✅ 已实现 | 单元测试 |
| <js> 内联 JS | evalJS | ⚠️ 有沙箱但 java.* 方法未全验证 | reader-js 79 方法 |
| @js: 规则前缀 | evalJS | ⚠️ 同上 | |
| MultiRule (&&/\|\|/%%) | splitSourceRule | ❌ blocker（CSS 路径未拆分） | release-blockers.json |
| 规则补全 (RuleComplete) | RuleComplete.kt | ❌ 未实现 | 0 代码 |
| 规则缓存 | splitSourceRuleCacheString | ❌ 未实现 | |

### 1.2 URL 构造（AnalyzeUrl.kt, 30+ 方法）

| 能力 | Legado 实现 | Reader Core 状态 | 证据 |
|------|------------|-----------------|------|
| {{key}}/{{page}} 模板展开 | replaceKeyPageJs | ✅ | auto_build_search 测试 |
| url,{json} DSL 格式 | analyzeUrl | ✅ | auto_build_search 测试 |
| POST/GET/HEAD 方法 | setMethod | ✅ | auto_build_search 测试 |
| charset 编码 (GBK/UTF-8) | encodeParams | ⚠️ Core 产出提示，Host 执行 | 未端到端验证 |
| header 合并 (source + DSL) | analyzeFields | ✅ | 单元测试 |
| Cookie 传递 | setCookie/saveCookie | ⚠️ Core 产出提示，Host 执行 | |
| JS URL 构造 (@js:/<js>) | analyzeJs | ⚠️ 有沙箱但 java.get/post 未验证 | |
| 重定向跟随 | OkHttp redirect | ❌ Core 不处理，Host 侧 | |
| 文件上传 (multipart) | upload | ❌ 未实现 | 0 代码 |
| GlideUrl (图片加载) | getGlideUrl | ❌ 未实现 | |

### 1.3 书源生命周期（model/webBook/）

| 能力 | Legado 实现 | Reader Core 状态 | 证据 |
|------|------------|-----------------|------|
| 搜索 (WebBook.searchBook) | BookList.kt | ✅ protocol book.search | 3 源 fixture |
| 详情 (WebBook.getBookInfo) | BookInfo.kt | ✅ protocol book.detail | 3 源 fixture |
| 目录 (WebBook.getChapterList) | BookChapterList.kt | ✅ protocol book.toc | 3 源 fixture |
| 正文 (WebBook.getBookContent) | BookContent.kt | ✅ protocol chapter.content | 3 源 fixture |
| 发现 (WebBook.exploreBook) | BookList.kt (explore) | ⚠️ 有 ruleExplore 解析但无 explore 协议方法 | 0 真实源 |
| 多页加载 (nextPage/nextTocUrl) | BookList/BookChapterList | ❌ 未实现 | 0 代码 |
| 段评 (ReviewRule) | ruleReview | ❌ 未实现 | 0 代码 |
| 书源校验 (CheckSource) | CheckSource.kt + Service | ❌ 未实现 | 0 代码 |
| 书源调试 (Debug) | Debug.kt + WebSocket | ❌ 未实现 | 0 代码 |

### 1.4 JS 扩展方法（JsExtensions.kt, 79 个方法）

| 分类 | 方法数 | Reader Core 状态 | 证据 |
|------|--------|-----------------|------|
| HTTP (ajax/ajaxAll/connect/get/post/head) | 6 | ⚠️ 有 java.get/post 但端到端未验证 | reader-js 79/79 单元测试 |
| WebView (webView/startBrowser/getVerificationCode) | 5 | ❌ Core 不碰 WebView（红线 4），需 Host | |
| 文件操作 (getFile/readFile/unzipFile/unrarFile/un7zFile) | 14 | ❌ 未实现 | 0 代码 |
| 编码 (base64/hex/encodeURI/bytesToStr) | 12 | ✅ 大部分已实现 | reader-js 单元测试 |
| Cookie (getCookie) | 2 | ⚠️ 有协议 cookie.get/set 但 JS 内调用未验证 | |
| 字体反混淆 (queryTTF/replaceFont) | 4 | ❌ 未实现 | 0 代码 |
| 繁简转换 (t2s/s2t) | 2 | ❌ 未实现 | 0 代码 |
| 时间格式化 (timeFormat/timeFormatUTC) | 2 | ✅ | |
| 其他 (toast/log/randomUUID/androidId/openUrl) | 6 | ⚠️ 部分 | |
| 缓存 (cacheFile/downloadFile) | 4 | ❌ 未实现 | 0 代码 |
| 脚本导入 (importScript) | 1 | ❌ 未实现 | 0 代码 |

---

## 2. 本地书（model/localBook/）

| 格式 | Legado 实现 | Reader Core 状态 | 证据 |
|------|------------|-----------------|------|
| TXT | TextFile.kt | ✅ reader-local-book/txt.rs | crate test |
| EPUB | EpubFile.kt | ✅ reader-local-book/epub.rs | crate test |
| PDF | PdfFile.kt | ✅ reader-local-book/pdf.rs | crate test |
| Mobi | MobiFile.kt | ⚠️ reader-local-book/mobi.rs 有但未验证 | 0 真实文件 |
| Umd | UmdFile.kt | ❌ 未实现 | 0 代码 |
| TXT 目录规则 (TxtTocRule) | TxtTocRule.kt | ❌ 未实现 | 0 代码 |

---

## 3. RSS 订阅（model/rss/）

| 能力 | Legado 实现 | Reader Core 状态 | 证据 |
|------|------------|-----------------|------|
| RSS 源解析 (RssParserByRule) | RssParserByRule.kt | ✅ reader-rss | crate test |
| 默认 RSS 解析 (RssParserDefault) | RssParserDefault.kt | ⚠️ 不确定 | |
| RSS 文章内容 (ruleContent) | Rss.kt | ⚠️ 部分 | |
| RSS 收藏 (RssStar) | RssStar entity | ❌ 未实现 | 0 代码 |
| RSS 阅读记录 (RssReadRecord) | RssReadRecord entity | ❌ 未实现 | 0 代码 |
| RSS 订阅管理 (subscription) | ui/rss/subscription | ❌ UI 层，不在 Core | |

---

## 4. TTS 朗读（help/TTS.kt + service/）

| 能力 | Legado 实现 | Reader Core 状态 | 证据 |
|------|------------|-----------------|------|
| 系统 TTS 发声 | TextToSpeech | ❌ Core 不发声（红线），Host 侧 | |
| TTS 队列状态机 | TTSReadAloudService | ✅ reader-runtime/tts.rs | conformance |
| 文本切片 | TTS.kt | ✅ tts.slice protocol | conformance |
| HttpTTS (在线语音) | HttpTTS entity + HttpReadAloudService | ❌ 未实现 | 0 代码 |
| TTS 配置 (engine/rate/followSys) | AppConfig | ❌ 未实现 | |
| 蓝牙按键 (MediaButton) | MediaButtonReceiver | ❌ Host 侧 | |

---

## 5. 同步与备份（help/storage/ + lib/webdav/）

| 能力 | Legado 实现 | Reader Core 状态 | 证据 |
|------|------------|-----------------|------|
| WebDAV 同步 | AppWebDav + WebDav.kt | ✅ reader-sync/webdav | crate test |
| 备份 (JSON + AES) | Backup.kt + BackupAES.kt | ⚠️ sync.backup protocol 有但 AES 未验证 | |
| 恢复 | Restore.kt | ⚠️ sync.merge protocol 有但未验证 | |
| 备份配置 | BackupConfig.kt | ❌ 未实现 | |
| 旧数据迁移 | ImportOldData.kt | ❌ 不需要 | |

---

## 6. 书架与数据管理（data/entities/）

| 实体 | Legado 实现 | Reader Core 状态 | 证据 |
|------|------------|-----------------|------|
| Book | Book.kt | ✅ reader-domain | |
| BookChapter | BookChapter.kt | ✅ reader-domain | |
| BookSource | BookSource.kt | ✅ reader-domain | |
| BookGroup (书架分组) | BookGroup.kt | ❌ 未实现 | 0 代码 |
| Bookmark (书签) | Bookmark.kt | ❌ 未实现 | 0 代码 |
| ReplaceRule (替换规则) | ReplaceRule.kt | ❌ 未实现 | 0 代码 |
| TxtTocRule | TxtTocRule.kt | ❌ 未实现 | 0 代码 |
| DictRule (字典规则) | DictRule.kt | ❌ 未实现 | 0 代码 |
| HttpTTS | HttpTTS.kt | ❌ 未实现 | 0 代码 |
| RssSource | RssSource.kt | ✅ reader-domain | |
| RssArticle | RssArticle.kt | ✅ reader-domain | |
| Cookie | Cookie.kt | ✅ cookie.get/set protocol | |
| Cache | Cache.kt | ✅ cache.get/put protocol | |
| ReadRecord (阅读时长) | ReadRecord.kt | ❌ 未实现 | 0 代码 |
| SearchBook (搜索历史) | SearchBook.kt | ❌ 未实现 | 0 代码 |
| SearchKeyword | SearchKeyword.kt | ❌ 未实现 | 0 代码 |
| RuleSub (规则订阅) | RuleSub.kt | ❌ 未实现 | 0 代码 |
| BookChapterReview (段评) | BookChapterReview.kt | ❌ 未实现 | 0 代码 |

---

## 7. 内容处理（help/book/）

| 能力 | Legado 实现 | Reader Core 状态 | 证据 |
|------|------------|-----------------|------|
| 替换规则 (ContentProcessor) | ContentProcessor.kt | ❌ 未实现 | 0 代码 |
| 替换规则分析 (ReplaceAnalyzer) | ReplaceAnalyzer.kt | ❌ 未实现 | 0 代码 |
| 内容净化 | ContentHelp.kt | ❌ 未实现 | 0 代码 |
| 繁简转换 | ChineseUtils.kt | ❌ 未实现 | 0 代码 |
| 去除同名标题 | ContentProcessor.upRemoveSameTitle | ❌ 未实现 | 0 代码 |
| 智能段落修正 | ContentHelp.kt | ❌ 未实现 | 0 代码 |

---

## 8. 阅读引擎（ui/book/read/）— Host/UI 层

| 能力 | Legado 实现 | Reader Core 状态 | 备注 |
|------|------------|-----------------|------|
| 翻页动画 (Cover/Slide/Simulation/Scroll/None/Horizontal) | page/delegate/ 7 种 | N/A | Host/UI 层 |
| 文字排版 (字号/行距/段距/缩进/粗体) | ReadBookConfig 224 配置项 | N/A | Host/UI 层 |
| 背景设置 (颜色/图片/EInk) | ReadBookConfig | N/A | Host/UI 层 |
| 主题 (白天/夜间/EInk) | ThemeConfig | N/A | Host/UI 层 |
| 点击区域配置 | AppConfig 9 区域 | N/A | Host/UI 层 |
| 自动阅读 | AutoReadDialog | N/A | Host/UI 层 |
| 全文搜索 | SearchContentViewModel | ❌ 未实现 | Core 侧 0 代码 |
| 内容编辑 | ContentEditDialog | ❌ 未实现 | |
| 换源 | ChangeBookSourceDialog | ❌ 未实现 | |
| 换封面 | ChangeCoverDialog | ❌ 未实现 | |

---

## 9. 图片与封面（model/ImageProvider + model/BookCover）

| 能力 | Legado 实现 | Reader Core 状态 | 证据 |
|------|------------|-----------------|------|
| 封面加载 | BookCover.kt | N/A | Host 侧 |
| 封面规则搜索 | searchCover | ❌ 未实现 | 0 代码 |
| 封面解密 (coverDecodeJs) | ImageUtils + coverDecodeJs | ❌ 未实现 | 0 代码 |
| 图片缓存 | ImageProvider.BitmapLruCache | N/A | Host 侧 |
| 漫画图片 | ReadManga.kt | ❌ 未实现 | 0 代码 |

---

## 10. 其他系统能力

| 能力 | Legado 实现 | Reader Core 状态 | 备注 |
|------|------------|-----------------|------|
| Web 服务 (HttpServer + WebSocket) | web/ | ❌ 未实现 | Core 不开 socket（红线 4） |
| ContentProvider API | api/controller/ | ❌ 未实现 | |
| 书源/规则导入导出 | ui/association/ 7 种导入 | ❌ 未实现 | |
| 后台服务 (Service) | 7 个 Service | N/A | Host 侧 |
| 后台任务/通知 | Service + Notification | N/A | Host 侧 |
| 媒体键 (MediaButton) | MediaButtonReceiver | N/A | Host 侧 |

---

## 11. 能力覆盖率总结

### Legado 全部能力模块统计

| 大类 | 子能力数 | Reader 已实现 | 部分实现 | 未实现 | 不适用(Host层) |
|------|---------|-------------|---------|--------|--------------|
| 书源解析引擎 | 28 | 8 | 12 | 8 | 0 |
| 本地书 | 6 | 3 | 1 | 2 | 0 |
| RSS | 6 | 2 | 1 | 3 | 0 |
| TTS | 6 | 2 | 0 | 4 | 0 |
| 同步备份 | 5 | 2 | 2 | 1 | 0 |
| 书架数据实体 | 18 | 5 | 0 | 13 | 0 |
| 内容处理 | 6 | 0 | 0 | 6 | 0 |
| 阅读引擎 | 10 | 0 | 0 | 2 | 8 |
| 图片封面 | 5 | 0 | 0 | 3 | 2 |
| 其他系统 | 7 | 0 | 0 | 3 | 4 |
| **合计** | **97** | **22** | **16** | **45** | **14** |

### 诚实评估

- **Core 侧实际完成度: ~23%**（22/83 非Host能力，其中大部分只有单元测试无真实源验证）
- **部分实现 16 项中，大部分从未用真实 Legado 数据验证过**
- **45 项完全未实现**（替换规则/繁简/TXT目录/书签/书架分组/段评/多页/发现/换源/封面规则/字体反混淆/规则补全/规则订阅/阅读记录/搜索历史等）
- **之前声称的 "S1/S3/S5 100%" 严重高估** — 没有对照 Legado 全部能力清单做过系统验证
- **459 源集合的真实通过率 = 未知** — 从未批量测试过

### "能力不差于 Legado" 的验收缺口

即使 459 源 L1-L5 全绿，仍有大量 Legado 能力未对标:
1. 多页加载（nextPage/nextTocUrl）— 大量源有翻页
2. 替换规则 — Legado 核心内容处理能力
3. 繁简转换 — JsExtensions.t2s/s2t
4. TXT 目录规则 — 本地书核心能力
5. 书签/书架分组/阅读记录 — 数据管理基础
6. 段评 — 书源 ruleReview
7. 发现（explore）— 书源 exploreUrl
8. 字体反混淆（queryTTF）— 反爬能力
9. 封面解密（coverDecodeJs）— 反爬能力
10. 规则补全（RuleComplete）— 源兼容性
11. 全文搜索 — 阅读体验
12. 换源 — 阅读体验

---

*本文为 Legado 能力清单审计文档，是 "能力底线 = Legado" 的验收基准。
任何 "能力已完成" 的判断必须对照本清单逐项验证。*
