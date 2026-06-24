# Reader-Core-Native Feature Matrix

> 唯一来源：所有能力分配以此文档为准。
> 旧 `FEATURE_MATRIX`、`CAPABILITY_MATRIX`、`LEGADO_*`、`ANDROID_*_CAPABILITY*` 等文档均已归档至各仓库 `_archived_planning_2026-06-24/`。

## 能力归属总表

| 能力 | Rust Core | Platform Adapter | 暂缓 | 退役 |
|------|:---------:|:----------------:|:----:|:----:|
| Book/Chapter/Source 数据模型 | ✅ | | | |
| CSS Selector 规则 | ✅ | | | |
| XPath 规则 | ✅ | | | |
| JSONPath 规则 | ✅ | | | |
| 正则规则 | ✅ | | | |
| `@` 链规则 | ✅ | | | |
| 变量作用域 | ✅ | | | |
| 多字段规则 | ✅ | | | |
| 替换规则 | ✅ | | | |
| bookList scoping | ✅ | | | |
| tag.index | ✅ | | | |
| JS 执行 (QuickJS) | ✅ | | | |
| JS host API (console, fetch, crypto, etc.) | ✅ | | | |
| 请求参数构建 | ✅ | | | |
| 重定向策略控制 | ✅ | | | |
| Cookie 策略控制 | ✅ | | | |
| TLS / 实际网络 socket | | ✅ | | |
| HTTP Transport | | ✅ | | |
| 响应编码检测和转换 | ✅ | | | |
| HTML 解析 | ✅ | | | |
| XML 解析 | ✅ | | | |
| JSON 解析 | ✅ | | | |
| 内容清洗和标准化 | ✅ | | | |
| 搜索规则 | ✅ | | | |
| 书籍详情规则 | ✅ | | | |
| 目录规则 | ✅ | | | |
| 正文规则 | ✅ | | | |
| 书源导入/导出 | ✅ | | | |
| SQLite schema 管理 | ✅ | | | |
| 数据库迁移 | ✅ | | | |
| 章节内容缓存 | ✅ | | | |
| 阅读进度 | ✅ | | | |
| 下载队列 | ✅ | | | |
| 最近历史 | ✅ | | | |
| Cookie/Session 持久化 | ✅ | | | |
| Recovery/校验/Diff | ✅ | | | |
| TXT 解析 | ✅ | | | |
| EPUB 解析 | ✅ | | | |
| RSS 解析和订阅状态 | ✅ | | | |
| WebDAV 协议和冲突策略 | ✅ | | | |
| 备份/恢复 | ✅ | | | |
| 同步/冲突解决 | ✅ | | | |
| TTS 文本切片和播放队列 | ✅ | | | |
| 系统 TTS 发声 | | ✅ | | |
| 登录 WebView 交互 | | ✅ | | |
| WebView Cookie 获取 | | ✅ | | |
| 安全凭据存储 (Keychain/Keystore/etc) | | ✅ | | |
| 文件选择和沙箱授权 | | ✅ | | |
| UI 组件和导航 | | ✅ | | |
| 主题和字体 | | ✅ | | |
| 后台任务和通知 | | ✅ | | |
| 包体签名和分发 | | ✅ | | |

## V1 功能边界

V1 交付物（搜索→详情→目录→正文 完整链）：

- [ ] 书源导入
- [ ] 搜索
- [ ] 书籍详情
- [ ] 目录
- [ ] 正文阅读
- [ ] 章节缓存
- [ ] 阅读进度
- [ ] TXT 基础支持
- [ ] EPUB 基础支持

## 退役清单

各平台独立实现将在 Rust Core 对应模块完成后退役：

- [ ] Android: Room 内容数据库 → Rust SQLite
- [ ] Android: HTML/XML parser → Rust parser
- [ ] Android: RSS parser → Rust RSS
- [ ] Android: TXT/EPUB parser → Rust local-book
- [ ] Android: WebDAV/sync 逻辑 → Rust sync
- [ ] Android: remote cache/offline → Rust storage
- [ ] iOS: Swift Reader-Core 运行依赖 → Rust Core
- [ ] HarmonyOS: 独立非 UI 实现 → Rust Core

---

*最后更新: 2026-06-24 | 以 `ARCHITECTURE.md` 为准*
