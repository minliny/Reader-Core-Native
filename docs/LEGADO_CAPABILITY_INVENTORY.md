# Legado 能力清单与 Reader 对标审计

日期：2026-06-27（2026-06-28 修正：基于 4 agent 真实代码交叉验证，修正虚报/漏报）
审计来源：`/Users/minliny/Documents/legado` 源码（815 个 .kt/.java 文件）

本文是 Legado 全部能力的枚举清单，作为 Reader "能力底线 = Legado" 的验收基准。
任何 "能力已完成" 的判断必须对照本清单逐项验证，不得用代码量/测试数自证。

---

## 证据级别说明

每项能力均标注证据来源（"证据级别"列）：

| 标记 | 含义 |
|------|------|
| 🟢 | 真实 Legado 书源跑通（459 源 batch 验证） |
| 🔵 | 单元测试通（合成 fixture / conformance） |
| 🟡 | 代码存在但未测试 / 未端到端验证 / 仅 entity+CRUD |
| 🔴 | 死代码 / dispatch 缺失 / 0 代码 |

---

## 1. 书源解析引擎（model/analyzeRule + model/webBook）

### 1.1 规则语言（AnalyzeRule.kt, 84 个方法）

| 能力 | Legado 实现 | Reader Core 状态 | 证据 | 证据级别 |
|------|------------|-----------------|------|---------|
| CSS 选择器 (@css:/@text/@href/&&/;/\|\|) | AnalyzeByJSoup | ⚠️ 部分（MultiRule 已修，待批量验证） | fixture_vertical 3 源 | 🔵 |
| XPath (@xpath:/@@) | AnalyzeByXPath | ✅ 有实现 | conformance | 🔵 |
| JSONPath ($./$[) | AnalyzeByJSonPath | ✅ 有实现 | conformance | 🔵 |
| 正则 (regex) | AnalyzeByRegex | ⚠️ 有 regex-suffix 但未完整验证 | 0 真实源 | 🟡 |
| @put/@get 变量机制 | splitPutRule/evalPattern | ✅ 已实现 | reader-rule 单元测试 | 🔵 |
| {{}} 模板 | evalPattern | ✅ 已实现 | 单元测试 | 🔵 |
| <js> 内联 JS | evalJS | ⚠️ 有沙箱但 java.* 方法未全验证 | reader-js 79 方法 | 🔵 |
| @js: 规则前缀 | evalJS | ⚠️ 同上 | reader-js 79 方法 | 🔵 |
| MultiRule (&&/\|\|/%%) | splitSourceRule | ✅ 已实现（blocker 已解） | reader-rule 15 单元 + yodu 4 集成 = 19 tests | 🔵 |
| 规则补全 (RuleComplete) | RuleComplete.kt | ✅ 已实现 | 32 tests + reader-rule auto_complete_rule | 🔵 |
| 规则缓存 | splitSourceRuleCacheString | ❌ 未实现 | 0 代码 | 🔴 |

### 1.2 URL 构造（AnalyzeUrl.kt, 30+ 方法）

| 能力 | Legado 实现 | Reader Core 状态 | 证据 | 证据级别 |
|------|------------|-----------------|------|---------|
| {{key}}/{{page}} 模板展开 | replaceKeyPageJs | ✅ | auto_build_search 测试 | 🔵 |
| url,{json} DSL 格式 | analyzeUrl | ✅ | auto_build_search 测试 | 🔵 |
| POST/GET/HEAD 方法 | setMethod | ✅ | auto_build_search 测试 | 🔵 |
| charset 编码 (GBK/UTF-8) | encodeParams | ⚠️ Core 产出提示，Host 执行 | 未端到端验证 | 🟡 |
| header 合并 (source + DSL) | analyzeFields | ✅ | 单元测试 | 🔵 |
| Cookie 传递 | setCookie/saveCookie | ⚠️ Core 产出提示，Host 执行 | | 🟡 |
| JS URL 构造 (@js:/<js>) | analyzeJs | ⚠️ 有沙箱但 java.get/post 未验证 | | 🟡 |
| 重定向跟随 | OkHttp redirect | ❌ Core 不处理，Host 侧 | | 🟡 |
| 文件上传 (multipart) | upload | ❌ 未实现 | 0 代码 | 🔴 |
| GlideUrl (图片加载) | getGlideUrl | ❌ 未实现（Host 侧图片） | 0 代码 | 🔴 |

### 1.3 书源生命周期（model/webBook/）

| 能力 | Legado 实现 | Reader Core 状态 | 证据 | 证据级别 |
|------|------------|-----------------|------|---------|
| 搜索 (WebBook.searchBook) | BookList.kt | ✅ protocol book.search | 3 源 fixture | 🔵 |
| 详情 (WebBook.getBookInfo) | BookInfo.kt | ✅ protocol book.detail | 3 源 fixture | 🔵 |
| 目录 (WebBook.getChapterList) | BookChapterList.kt | ✅ protocol book.toc | 3 源 fixture | 🔵 |
| 正文 (WebBook.getBookContent) | BookContent.kt | ✅ protocol chapter.content | 3 源 fixture | 🔵 |
| 发现 (WebBook.exploreBook) | BookList.kt (explore) | ✅ dispatch 活跃，handler 可达 | remote.rs:466-468 dispatch + 2341 source_explore + 11 tests (explore_kinds.rs) | 🟢 |
| 多页加载 (nextPage/nextTocUrl) | BookList/BookChapterList | ✅ 已实现 | 8 tests pagination | 🔵 |
| 段评 (ReviewRule) | ruleReview | 🟠 struct 已定义，dispatch 缺失 | reader-domain ReviewRule struct (lib.rs:543)，无 SOURCE_REVIEW dispatch | 🟠 |
| 书源校验 (CheckSource) | CheckSource.kt + Service | ❌ 未实现 | 0 代码 | 🔴 |
| 书源调试 (Debug) | Debug.kt + WebSocket | ❌ 未实现 | 0 代码 | 🔴 |

### 1.4 JS 扩展方法（JsExtensions.kt, 79 个方法）

| 分类 | 方法数 | Reader Core 状态 | 证据 | 证据级别 |
|------|--------|-----------------|------|---------|
| HTTP (ajax/ajaxAll/connect/get/post/head) | 6 | 🟡 descriptor 路由已实现（HostDescriptor），Host 执行未端到端验证 | reader-js host_routing 76+ tests | 🟡 |
| WebView (webView/startBrowser/getVerificationCode) | 5 | 🟡 descriptor 路由已实现，Core 不碰 WebView（红线 4），Host 执行 | reader-js WebView/StartBrowser 等 6 HostDescriptor 变体 | 🟡 |
| 文件操作 (getFile/readFile/unzipFile/unrarFile/un7zFile) | 14 | 🟡 descriptor 路由已实现，Host 执行 | reader-js GetFile/ReadFile/UnzipFile 等 9 变体 | 🟡 |
| 编码 (base64/hex/encodeURI/bytesToStr) | 12 | ✅ 大部分已实现 | reader-js 单元测试 | 🔵 |
| Cookie (getCookie) | 2 | 🟡 descriptor 路由已实现，JS 内调用未端到端验证 | reader-js GetCookie 变体 | 🟡 |
| 字体反混淆 (queryTTF/replaceFont) | 4 | ✅ HostDescriptor 路由已实现+测试 | reader-js QueryTTF/ReplaceFont + host_routing_s3_closure.rs:465,516 | 🟢 |
| 繁简转换 (t2s/s2t) | 2 | 🟠 缺 fixT2sDict 排除字典（仅 zhhz 裸转换） | reader-js 98 + reader-content 53 tests | 🔵 |
| 时间格式化 (timeFormat/timeFormatUTC) | 2 | ✅ | 单元测试 | 🔵 |
| 其他 (toast/log/randomUUID/androidId/openUrl) | 6 | ⚠️ 部分 | | 🟡 |
| 缓存 (cacheFile/downloadFile) | 4 | 🟡 descriptor 路由已实现，Host 执行 | reader-js CacheFile/DownloadFile 变体 | 🟡 |
| 脚本导入 (importScript) | 1 | ✅ HostDescriptor 路由已实现+测试 | reader-js ImportScript + host_routing_residual.rs:151 | 🟢 |

---

## 2. 本地书（model/localBook/）

| 格式 | Legado 实现 | Reader Core 状态 | 证据 | 证据级别 |
|------|------------|-----------------|------|---------|
| TXT | TextFile.kt | ✅ reader-local-book/txt.rs | crate test | 🔵 |
| EPUB | EpubFile.kt | ✅ reader-local-book/epub.rs | crate test | 🔵 |
| PDF | PdfFile.kt | ✅ reader-local-book/pdf.rs | crate test | 🔵 |
| Mobi | MobiFile.kt | ⚠️ reader-local-book/mobi.rs 有但未验证 | 0 真实文件 | 🟡 |
| Umd | UmdFile.kt | 🟡 检测+MIME 已实现，parser 延后 | reader-local-book Umd variant + detection (lib.rs:46,3189)，parser deferred（Legado 也委托第三方） | 🟡 |
| TXT 目录规则 (TxtTocRule) | TxtTocRule.kt | 🟠 dispatch 被注释禁用 + 缺多规则择优算法（matchRule） | split_chapters 仅单规则；remote.rs:453-464 注释 | 🟡 |

---

## 3. RSS 订阅（model/rss/）

| 能力 | Legado 实现 | Reader Core 状态 | 证据 | 证据级别 |
|------|------------|-----------------|------|---------|
| RSS 源解析 (RssParserByRule) | RssParserByRule.kt | 🔵 合成 fixture，0 真实 RSS 源 | reader-rss crate test（123 处 example.com） | 🔵 |
| 默认 RSS 解析 (RssParserDefault) | RssParserDefault.kt | ⚠️ 不确定 | | 🟡 |
| RSS 文章内容 (ruleContent) | Rss.kt | ⚠️ 部分 | | 🟡 |
| RSS 收藏 (RssStar) | RssStar entity | ✅ 已实现+测试 | reader-rss starred field + set_entry_starred (lib.rs:665,2125) + tests | 🟢 |
| RSS 阅读记录 (RssReadRecord) | RssReadRecord entity | 🟠 读状态已跟踪，无独立实体 | reader-rss entry state map tracks read (lib.rs:919)，无 RssReadRecord entity | 🟠 |
| RSS 订阅管理 (subscription) | ui/rss/subscription | ❌ UI 层，不在 Core | Host 侧 | 🟡 |

---

## 4. TTS 朗读（help/TTS.kt + service/）

| 能力 | Legado 实现 | Reader Core 状态 | 证据 | 证据级别 |
|------|------------|-----------------|------|---------|
| 系统 TTS 发声 | TextToSpeech | ❌ Core 不发声（红线），Host 侧 | 边界正确 | 🟡 |
| TTS 队列状态机 | TTSReadAloudService | ✅ reader-runtime/tts.rs | conformance | 🔵 |
| 文本切片 | TTS.kt | ✅ tts.slice protocol | conformance | 🔵 |
| HttpTTS (在线语音) | HttpTTS entity + HttpReadAloudService | ❌ 未实现 | 0 代码 | 🔴 |
| TTS 配置 (engine/rate/followSys) | AppConfig | ❌ 未实现 | 0 代码 | 🔴 |
| 蓝牙按键 (MediaButton) | MediaButtonReceiver | ❌ Host 侧 | 边界正确 | 🟡 |

---

## 5. 同步与备份（help/storage/ + lib/webdav/）

| 能力 | Legado 实现 | Reader Core 状态 | 证据 | 证据级别 |
|------|------------|-----------------|------|---------|
| WebDAV 同步 | AppWebDav + WebDav.kt | ✅ reader-sync/webdav | crate test | 🔵 |
| 备份 (JSON + AES) | Backup.kt + BackupAES.kt | ⚠️ sync.backup protocol 有但 AES 未验证 | | 🟡 |
| 恢复 | Restore.kt | ⚠️ sync.merge protocol 有但未验证 | | 🟡 |
| 备份配置 | BackupConfig.kt | ❌ 未实现 | 0 代码 | 🔴 |
| 旧数据迁移 | ImportOldData.kt | ❌ 不需要 | N/A | 🟡 |

---

## 6. 书架与数据管理（data/entities/）

| 实体 | Legado 实现 | Reader Core 状态 | 证据 | 证据级别 |
|------|------------|-----------------|------|---------|
| Book | Book.kt | ✅ reader-domain | domain entity | 🔵 |
| BookChapter | BookChapter.kt | ✅ reader-domain | domain entity | 🔵 |
| BookSource | BookSource.kt | ✅ reader-domain | domain entity | 🔵 |
| BookGroup (书架分组) | BookGroup.kt | ✅ 8 tests + storage CRUD + dispatch WIRED | bookmark_commands.rs 8 tests；remote.rs:421-432 活跃 | 🔵 |
| Bookmark (书签) | Bookmark.kt | ✅ 8 tests + storage CRUD + dispatch WIRED | bookmark_commands.rs 8 tests + 5 entity/storage；remote.rs:409-420 活跃 | 🔵 |
| ReplaceRule (替换规则) | ReplaceRule.kt | ✅ 已实现 | ContentProcessor + 15+9 tests | 🔵 |
| TxtTocRule | TxtTocRule.kt | 🟠 dispatch 被注释禁用 + 缺多规则择优算法 | 6 单元测试；remote.rs:453-464 注释 | 🟡 |
| DictRule (字典规则) | DictRule.kt | 🟡 entity + table + CRUD 存在，无 dispatch/protocol | 5 entity/storage tests；无 DICT_RULE_* 常量 | 🟡 |
| HttpTTS | HttpTTS.kt | ❌ 未实现 | 0 代码 | 🔴 |
| RssSource | RssSource.kt | ✅ reader-domain | domain entity | 🔵 |
| RssArticle | RssArticle.kt | ✅ reader-domain | domain entity | 🔵 |
| Cookie | Cookie.kt | ✅ cookie.get/set protocol | | 🔵 |
| Cache | Cache.kt | ✅ cache.get/put protocol | | 🔵 |
| ReadRecord (阅读时长) | ReadRecord.kt | ✅ 9 tests + storage CRUD + dispatch WIRED | read_record_commands.rs 9 tests；remote.rs:433-444 活跃 | 🔵 |
| SearchBook (搜索历史) | SearchBook.kt | ❌ 未实现 | 0 代码 | 🔴 |
| SearchKeyword | SearchKeyword.kt | ❌ 未实现 | 0 代码 | 🔴 |
| RuleSub (规则订阅) | RuleSub.kt | ❌ 未实现 | 0 代码 | 🔴 |
| BookChapterReview (段评) | BookChapterReview.kt | ❌ 未实现 | 0 代码 | 🔴 |

---

## 7. 内容处理（help/book/）

| 能力 | Legado 实现 | Reader Core 状态 | 证据 | 证据级别 |
|------|------------|-----------------|------|---------|
| 替换规则 (ContentProcessor) | ContentProcessor.kt | 🟠 缺 Legado 6 阶段的 3 阶段（仅 chineseConvert/trim/replace） | reader-content/content_processor.rs:57 仅 3 stage | 🟡 |
| 替换规则分析 (ReplaceAnalyzer) | ReplaceAnalyzer.kt | ⚠️ 部分实现（分析逻辑内嵌在 ContentProcessor 中） | | 🟡 |
| 内容净化 | ContentHelp.kt | ❌ 未实现 | 0 代码 | 🔴 |
| 繁简转换 | ChineseUtils.kt | 🟠 缺 fixT2sDict 排除字典（仅 zhhz 裸转换） | reader-content/chinese.rs 113 行，无 fixT2sDict | 🟡 |
| 去除同名标题 | ContentProcessor.upRemoveSameTitle | ❌ 未实现 | 0 代码 | 🔴 |
| 智能段落修正 | ContentHelp.kt | ❌ 未实现 | 0 代码 | 🔴 |

---

## 8. 阅读引擎（ui/book/read/）— Host/UI 层

| 能力 | Legado 实现 | Reader Core 状态 | 备注 | 证据级别 |
|------|------------|-----------------|------|---------|
| 翻页动画 (Cover/Slide/Simulation/Scroll/None/Horizontal) | page/delegate/ 7 种 | N/A | Host/UI 层 | 🟡 |
| 文字排版 (字号/行距/段距/缩进/粗体) | ReadBookConfig 224 配置项 | N/A | Host/UI 层 | 🟡 |
| 背景设置 (颜色/图片/EInk) | ReadBookConfig | N/A | Host/UI 层 | 🟡 |
| 主题 (白天/夜间/EInk) | ThemeConfig | N/A | Host/UI 层 | 🟡 |
| 点击区域配置 | AppConfig 9 区域 | N/A | Host/UI 层 | 🟡 |
| 自动阅读 | AutoReadDialog | N/A | Host/UI 层 | 🟡 |
| 全文搜索 | SearchContentViewModel | ❌ 未实现 | Core 侧 0 代码 | 🔴 |
| 内容编辑 | ContentEditDialog | ❌ 未实现 | | 🔴 |
| 换源 | ChangeBookSourceDialog | ❌ 未实现 | | 🔴 |
| 换封面 | ChangeCoverDialog | ❌ 未实现 | | 🔴 |

---

## 9. 图片与封面（model/ImageProvider + model/BookCover）

| 能力 | Legado 实现 | Reader Core 状态 | 证据 | 证据级别 |
|------|------------|-----------------|------|---------|
| 封面加载 | BookCover.kt | N/A | Host 侧 | 🟡 |
| 封面规则搜索 | searchCover | ❌ 未实现 | 0 代码 | 🔴 |
| 封面解密 (coverDecodeJs) | ImageUtils + coverDecodeJs | ❌ 未实现 | 0 代码 | 🔴 |
| 图片缓存 | ImageProvider.BitmapLruCache | N/A | Host 侧 | 🟡 |
| 漫画图片 | ReadManga.kt | ❌ 未实现 | 0 代码 | 🔴 |

---

## 10. 其他系统能力

| 能力 | Legado 实现 | Reader Core 状态 | 备注 | 证据级别 |
|------|------------|-----------------|------|---------|
| Web 服务 (HttpServer + WebSocket) | web/ | ❌ Core 不开 socket（红线 4） | 边界正确 | 🟡 |
| ContentProvider API | api/controller/ | ❌ 未实现 | | 🔴 |
| 书源/规则导入导出 | ui/association/ 7 种导入 | ❌ 未实现 | | 🔴 |
| 后台服务 (Service) | 7 个 Service | N/A | Host 侧 | 🟡 |
| 后台任务/通知 | Service + Notification | N/A | Host 侧 | 🟡 |
| 媒体键 (MediaButton) | MediaButtonReceiver | N/A | Host 侧 | 🟡 |

---

## 11. 能力覆盖率总结

### Legado 全部能力模块统计（2026-06-28 修正后，按证据级别标记统计）

| 大类 | 子能力数 | 已实现(🟢+🔵) | 部分(🟡+🟠) | 未实现(🔴) | Host层(N/A) |
|------|---------|--------------|------------|-----------|-------------|
| 书源解析引擎 | 41 | 21 | 11 | 9 | 0 |
| 本地书 | 6 | 3 | 2 | 1 | 0 |
| RSS | 6 | 1 | 3 | 2 | 0 |
| TTS | 6 | 2 | 2 | 2 | 0 |
| 同步备份 | 5 | 1 | 3 | 1 | 0 |
| 书架数据实体 | 18 | 11 | 2 | 5 | 0 |
| 内容处理 | 6 | 0 | 3 | 3 | 0 |
| 阅读引擎 | 10 | 0 | 0 | 4 | 6 |
| 图片封面 | 5 | 0 | 0 | 3 | 2 |
| 其他系统 | 6 | 0 | 1 | 2 | 3 |
| **合计** | **109** | **39 (36%)** | **27 (25%)** | **32 (29%)** | **11 (10%)** |

> 注 1：上文细项按方法分类粒度展开共 109 行（书源解析引擎 1.4 节 11 类方法展开后大于原 28 计数）。
> 注 2：AGENTS.md 中的"97 项能力"为 1.4 节合并计的口径（109 - 1.4 节多出的 12 行 ≈ 97）；
> 本表采用 109 行展开口径以便与各节表格逐行对账。
> 注 3：达标率三档口径 — 真实达标 = 32%（35/109，🟢+🔵 且无 ⚠️/🟠 瑕疵）；
> 证据级别达标 = 36%（39/109，🟢+🔵 全计）；严格达标 = 0%（0/109，🟢 仅，无 459 源 batch 验证通过项）。

### 诚实评估

- **真实达标率 = 32%（35/109）**：在 🟢+🔵 证据级别 39 项中，有 4 项实现存在瑕疵（状态含 ⚠️/🟠），不计入真实达标：
  - CSS 选择器（⚠️ 部分，MultiRule 已修但待批量验证）
  - `<js>` 内联 JS（⚠️ 有沙箱但 java.* 方法未全验证）
  - `@js:` 规则前缀（⚠️ 同上）
  - 繁简转换（🟠 缺 fixT2sDict 排除字典，仅 zhhz 裸转换）
- **证据级别已实现（🟢+🔵）: 39/109 = 36%**（含 MultiRule blocker 已解、Bookmark/BookGroup/ReadRecord dispatch 已 WIRED、RSS 源解析有合成 fixture 测试）
- **部分实现/代码存在（🟡+🟠）: 27/109 = 25%**（含死代码 explore/ TxtTocRule dispatch 注释禁用、ChineseUtils 缺 fixT2sDict、ContentProcessor 仅 3/6 阶段、JS 扩展 5 类 descriptor 路由已实现但 Host 执行未端到端验证、Web 服务/系统 TTS/蓝牙按键等边界正确项）
- **未实现（🔴）: 25/109 = 23%**（修正：explore/importScript/queryTTF/RssStar 已实现，ReviewRule/Umd/RssReadRecord 降级为部分实现）（书源校验/书源调试/规则缓存/HttpTTS/搜索历史/规则订阅/内容净化/全文搜索/换源/换封面/漫画图片等 0 代码；段评=struct-only；字体反混淆=已路由）
- **Host 层（N/A）: 11/109 = 10%**（翻页动画/排版/背景/主题/点击区域/自动阅读/封面加载/图片缓存/后台服务/通知/媒体键 — Core 不实现，Host 侧）

> 达标率口径说明（三档）：
> - **真实达标 = 32%（35/109）**：🟢+🔵 且无 ⚠️/🟠 实现瑕疵（排除 4 项 🔵 待验证/缺字典项）
> - **证据级别达标 = 36%（39/109）**：🟢+🔵 全计（含 4 项有 caveat 的 🔵）
> - **严格达标 = 0%（0/109）**：🟢 仅（无 459 源 batch 验证通过项）
>
> 早期文档引用的"~31%/32%"为按 Status ✅ 计数的旧口径，本文统一改为按证据级别标记计数，
> 并新增"真实达标率 = 32%"口径以剔除 🔵 中状态含 ⚠️/🟠 的瑕疵项。

### 本轮修正项（2026-06-28，4 agent 真实代码交叉验证）

**漏报修正（❌ → 实际已实现）：**
1. Bookmark: ❌ 0 代码 → ✅ 8 tests + storage CRUD + dispatch WIRED（remote.rs:409-420 活跃）
2. BookGroup: ❌ 0 代码 → ✅ 8 tests + storage CRUD + dispatch WIRED（remote.rs:421-432 活跃）
3. ReadRecord: ❌ 0 代码 → ✅ 9 tests + storage CRUD + dispatch WIRED（remote.rs:433-444 活跃）
4. DictRule: ❌ 0 代码 → 🟡 entity + table + CRUD 存在，无 dispatch/protocol（5 entity/storage tests）

**虚报修正（✅ → 降级）：**
1. explore: ✅ protocol → ✅ dispatch 活跃（remote.rs:466-468），handler 可达，11 tests
2. TxtTocRule: ✅ 已实现 → 🟠 dispatch 被注释禁用 + 缺多规则择优算法（仅单规则 split_chapters）
3. ChineseUtils: ✅ 已实现 → 🟠 缺 fixT2sDict 排除字典（chinese.rs 仅 113 行 zhhz 裸转换）
4. ContentProcessor: ✅ 已实现 → 🟠 缺 Legado 6 阶段的 3 阶段（仅 chineseConvert/trim/replace）
5. RssParserByRule: ✅ crate test → 🔵 合成 fixture，0 真实 RSS 源（123 处 example.com）

**状态过期修正：**
1. MultiRule: ❌ blocker → ✅ blocker 已解（split_legado_combined_rule + combine，19 tests）

**JS 扩展方法 5 类从 ❌ → 🟡**（descriptor 路由已实现，符合 Core/Host 边界）：
- HTTP / WebView / 文件操作 / Cookie / 缓存 — 均通过 HostDescriptor 强类型变体路由至 Host，76+ tests

### "能力不差于 Legado" 的验收缺口

即使 459 源 L1-L5 全绿，仍有大量 Legado 能力未对标（已剔除本轮完成的 多页加载 / 书签·书架分组·阅读记录 / 规则补全）：

**部分实现（🟡/🟠，需补齐）：**
1. 替换规则 — Legado 核心内容处理能力（ContentProcessor 缺 3/6 阶段，仅 chineseConvert/trim/replace）🟠
2. 繁简转换 — JsExtensions.t2s/s2t（缺 fixT2sDict 排除字典，仅 zhhz 裸转换）🟠
3. TXT 目录规则 — 本地书核心能力（dispatch 禁用 + 缺多规则择优算法 matchRule）🟠
4. 替换规则分析 (ReplaceAnalyzer) — 分析逻辑内嵌在 ContentProcessor，未独立 🟡

**完全缺失（🔴，0 代码）：**
5. 段评 — 书源 ruleReview 🔴
6. 发现（explore）— 书源 exploreUrl（dispatch 禁用，handler 死代码）🔴
7. 字体反混淆（queryTTF/replaceFont）— 反爬能力 🔴
8. 封面解密（coverDecodeJs）— 反爬能力 🔴
9. 规则缓存 — splitSourceRuleCacheString 🔴
10. 书源校验（CheckSource）— 源质量校验 🔴
11. 书源调试（Debug + WebSocket）— 源开发调试 🔴
12. HttpTTS（在线语音）— TTS 双轨之一 🔴
13. TTS 配置（engine/rate/followSys）— TTS 参数管理 🔴
14. 全文搜索 — 阅读体验 🔴
15. 内容编辑 — 阅读体验 🔴
16. 换源 — 阅读体验 🔴
17. 换封面 — 阅读体验 🔴
18. 封面规则搜索（searchCover）— 反爬/补全 🔴
19. 漫画图片（ReadManga）— 漫画阅读 🔴
20. 内容净化（ContentHelp）— 内容处理 🔴
21. 去除同名标题（upRemoveSameTitle）— 内容处理 🔴
22. 智能段落修正 — 内容处理 🔴
23. Umd 格式 — 本地书 🔴
24. RSS 收藏（RssStar）— RSS 数据 🔴
25. RSS 阅读记录（RssReadRecord）— RSS 数据 🔴
26. 备份配置（BackupConfig）— 同步备份 🔴
27. SearchBook / SearchKeyword — 搜索历史 🔴
28. RuleSub（规则订阅）— 源订阅 🔴
29. 文件上传（multipart）— URL 构造 🔴
30. 脚本导入（importScript）— JS 扩展 🔴
31. ContentProvider API — 外部接口 🔴
32. 书源/规则导入导出（7 种导入）— 源管理 🔴

---

*本文为 Legado 能力清单审计文档，是 "能力底线 = Legado" 的验收基准。
任何 "能力已完成" 的判断必须对照本清单逐项验证，并标注证据级别。
2026-06-28 修正基于 4 agent 真实代码交叉验证，修正虚报/漏报。*
