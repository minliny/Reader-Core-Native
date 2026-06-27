# iOS 退役旧 Swift Core 服务层 + 接入 Rust Core 为默认业务路径 (S6.2/S6.3) 实施方案

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 Reader for iOS 仓库 `codex/ios-real-app-core-evidence` 分支的业务路径从「默认 mock + 旧 Swift Core `ReaderCoreServiceFactory` 作为 real 路径」切换为「Rust Core C ABI 为唯一默认业务路径」,退役 `MockReaderCoreService` 与 `Default*Service` 旧 Swift Core 服务包装,保留 35 个 SwiftUI 页面与 Swift 模型层(`ReaderCoreModels`/`ReaderCoreProtocols`)不动,让 iOS 模拟器跑通 `search→detail→toc→content` 全链路(经 Rust Core,非 mock)。

**Architecture:** S6.1 已完成"桥接层就绪 + opt-in":`ReaderCoreNativeAdapter/` 提供 `ReaderCore.xcframework`(ios-arm64-simulator + macos-arm64),`RustCoreRuntimeHolder` 单例持有 runtime,`RustCoreSearchService/TOCService/ContentService` 实现 Swift `SearchService/TOCService/ContentService` 协议并 dispatch `book.search/book.toc/chapter.content` 到 Rust Core,`HostRequestRouter` 用 `URLSessionHTTPClient` 执行 Core 产出的 `host.request`。`ReaderApp.swift` 已在启动时 boot Rust Core + `configureRustCoreMode()`。**但** `ShellAssembly.makeDefaultReadingFlowCoordinator()` 仍默认返回 `makeMockReadingFlowCoordinator()`(用 `MockSearchService` 等 wrapper),且 `makeRealReadingFlowCoordinator()` 仍指向旧 Swift Core `ReaderCoreServiceFactory`。本方案:S6.2 让 Rust Core 成为 `ShellAssembly` 的显式默认 + 从生产路径退役 `MockReaderCoreService`;S6.3 退役旧 Swift Core 服务工厂路径 + 添加模拟器 E2E 证明。

**Tech Stack:** Rust Core(`reader-ffi` staticlib, ABI v1,通过 `ReaderCore.xcframework` 消费)/ Swift 5.9 / SwiftUI / Xcode 16 / iOS Simulator (iPhone 17 Pro, iOS 26.5)/ `URLSessionHTTPClient`(host HTTP transport)/ `HostRequestRouter`(host.request → host.complete 桥)/ `ReaderCoreModels`+`ReaderCoreProtocols`(Swift 模型与协议,保留)。

**Repos:**
- 主仓库(只读源 + plan 文档):`/Users/minliny/Documents/Reader-Core-Native`
- iOS 工作仓库:`/Users/minliny/Documents/Reader for iOS`
- 当前分支:`codex/ios-real-app-core-evidence`(最新 commit `44fd48e feat(ios/s6.1): wire Rust Core dispatch into iOS business path`)

**审计前置结论**(已完成,见下文):
- iOS binding 已就绪:`ReaderCore.xcframework` 存在,`ReaderCoreNativeRuntime.swift` 调 `rc_runtime_create/send/cancel/destroy`,`RustCoreSearchService/TOCService/ContentService.swift` 已实现并在 shell smoke 33/33 PASS
- S6.1 已在 `ReaderApp.swift` 启动时 boot Rust Core + `configureRustCoreMode()`,provider 模式默认切到 `.rustCore`
- `ShellAssembly.makeDefaultReadingFlowCoordinator(useReal:)` 仍默认返回 `makeMockReadingFlowCoordinator()`(line 101-111),`makeRealReadingFlowCoordinator()` 仍用旧 Swift Core `ReaderCoreServiceFactory`(line 47-71)
- `MockSearchService/MockTOCService/MockContentService`(ShellAssembly.swift line 127-212)是 thin delegate —— 调 `provider.searchBooks` 等,provider 在 `.rustCore` 模式下会路由到 RustCore*Service —— **命名误导**,实际不是 mock
- `MockReaderCoreService.swift`(CoreBridge/)是真正的 mock 数据生成器,生产路径不应再依赖
- `CoreIntegration/Default{Search,TOC,Content}Service.swift` + `DefaultBookSourceDecoder.swift` + `InMemoryBookSourceRepository.swift` 是旧 Swift Core `ReaderCoreServiceFactory` 的包装层,应退役
- 96 文件 import `ReaderCoreModels/ReaderCoreProtocols/ReaderCoreServices` —— **大部分只用模型类型**(BookSource/SearchResultItem/TOCItem/ContentPage),这些由 RustCore*Service 也消费,不能删
- 35 个 SwiftUI 页面(Features/)是纯 UI,不动
- 预存问题(STATUS.md Round 6/7):`ReaderApp` target 有 `BrightnessPolicy` 跨模块可见性问题,用独立 scheme `ReaderCoreNativeAdapterSmokeTests` 绕过,不修复

**红线**(章程 §4 / §10):
- Core 不开 socket、不碰 WebView、不存明文凭据
- iOS 仅保留 URLSession HTTP transport + WebView(本期不实现)+ Keychain + AVSpeech 在 Core/Host 边界
- 不破坏 35 个 SwiftUI 页面的纯 UI 代码
- 不删除 `ReaderCoreModels`/`ReaderCoreProtocols`(Swift 模型与协议,RustCore*Service 也消费)
- wrapper smoke ≠ device proof,分层标注证据
- 不破坏主仓库 dirty 文件(并发 agent 工作)

**验证命令**:
```bash
cd "/Users/minliny/Documents/Reader for iOS"
# 1. Shell smoke(基线,必绿)
bash iOS/ReaderCoreNativeAdapter/run-shell-smoke.sh
# 2. iOS-sim XCTest(基线,必绿)
xcodebuild -scheme ReaderCoreNativeAdapterSmokeTests \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro' test
# 3. App build(必绿)
xcodebuild build -project ReaderForIOS.xcodeproj -scheme ReaderForIOSApp \
  -configuration Debug -destination 'platform=iOS Simulator,name=iPhone 17 Pro'
# 4. App tests(若 scheme 支持)
xcodebuild test -project ReaderForIOS.xcodeproj -scheme ReaderForIOSApp \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro'
```

---

## 文件结构

### iOS 仓库新建文件

```
iOS/
├── CoreBridge/
│   └── RustCoreBookDetailService.swift              # 新建:book.detail dispatch via Rust Core(补 S6.1 缺口)
├── Tests/
│   └── ReaderAppTests/
│       └── RustCoreEndToEndTest.swift                # 新建:simulator E2E search→detail→toc→content via Rust Core
```

### iOS 仓库修改文件

```
iOS/
├── Shell/ShellAssembly.swift                         # 修改:makeDefault → Rust Core;retire makeReal old-factory path
├── App/ReaderApp.swift                               # 修改:移除 useRealServices UserDefaults 分支,显式 Rust Core
├── CoreBridge/ReaderCoreServiceProvider.swift        # 修改:默认 mode = .rustCore;移除 mock/real/controlledOnline 生产分支(保留测试可注入)
├── CoreBridge/RustCoreServiceSupport.swift           # 不动(已就绪)
├── CoreBridge/RustCoreSearchService.swift            # 不动(已就绪)
├── CoreBridge/RustCoreTOCService.swift               # 不动(已就绪)
├── CoreBridge/RustCoreContentService.swift           # 不动(已就绪)
```

### iOS 仓库退役文件(从生产路径移除,部分保留为测试 fixture)

```
iOS/CoreBridge/MockReaderCoreService.swift            # 保留文件,但 ShellAssembly 不再引用;仅测试可注入
iOS/CoreIntegration/DefaultSearchService.swift        # 退役(删除或标记 @available(*, deprecated))
iOS/CoreIntegration/DefaultTOCService.swift           # 退役
iOS/CoreIntegration/DefaultContentService.swift       # 退役
iOS/CoreIntegration/DefaultBookSourceDecoder.swift    # 保留(书源 JSON 解码,RustCore*Service 依赖 ReaderCoreModels 的 BookSource)
iOS/CoreIntegration/InMemoryBookSourceRepository.swift # 保留(书源仓库,UI 依赖)
iOS/CoreIntegration/ReadingFlowCoordinator.swift      # 保留(协调器,UI 依赖)
iOS/CoreIntegration/CoreLocalBookImportService.swift  # 保留(本地书,本期不动)
iOS/CoreIntegration/CoreRSSFeedService.swift          # 保留(RSS,本期不动)
```

### 主仓库改动

**零** —— 本期仅消费主仓库已就绪的 `bindings/ios/` + `include/reader_core.h` + 预构建 xcframework,不改主仓库任何文件。Plan 文档本身提交到主仓库 `docs/superpowers/plans/`。

---

## 任务分解

### Task 1: 让 Rust Core 成为 ShellAssembly 的显式默认

**Files:**
- Modify: `/Users/minliny/Documents/Reader for iOS/iOS/Shell/ShellAssembly.swift`

**背景**:当前 `makeDefaultReadingFlowCoordinator(useReal:)` 在 `useReal == false` 时返回 `makeMockReadingFlowCoordinator()`,而 `ReaderApp.swift` line 28-29 读 `UserDefaults.standard.bool(forKey: "useRealServices")`(默认 false)。虽然 `ReaderApp.swift` line 40-48 已 boot Rust Core + `configureRustCoreMode()`,且 `MockSearchService` 实际 delegate 到 provider(在 rustCore 模式下会路由到 RustCore*Service),但这个路径不显式、命名误导、且 `useRealServices` 开关会让用户误以为有"real"路径可选。本 Task 让 Rust Core 成为显式默认。

- [ ] **Step 1: 读 ShellAssembly.swift 当前状态确认 line 101-111**

```bash
cd "/Users/minliny/Documents/Reader for iOS"
sed -n '101,112p' iOS/Shell/ShellAssembly.swift
```
Expected: 看到 `makeDefaultReadingFlowCoordinator(useReal:)` 当前实现 —— `if useReal { return makeRealReadingFlowCoordinator() }` 否则 `return makeMockReadingFlowCoordinator()`。

- [ ] **Step 2: 修改 `makeDefaultReadingFlowCoordinator` 让 Rust Core 成为默认**

用 Edit 工具替换 `iOS/Shell/ShellAssembly.swift` 的 `makeDefaultReadingFlowCoordinator` 函数:

旧代码(line 101-111):
```swift
    public static func makeDefaultReadingFlowCoordinator(useReal: Bool = false) -> ReadingFlowCoordinator {
        // S6.1: Preserves prior mock/real semantics so existing shell smoke
        // tests stay green. Rust Core is an explicit opt-in via
        // makeRustCoreReadingFlowCoordinator() or via ReaderCoreServiceProvider
        // mode = .rustCore (business path switches at the provider level, not
        // by silently replacing the default coordinator wiring).
        if useReal {
            return makeRealReadingFlowCoordinator()
        }
        return makeMockReadingFlowCoordinator()
    }
```

新代码:
```swift
    /// S6.2: Rust Core is the explicit default business path.
    /// The legacy `useReal` flag is retained only for test injection (tests
    /// that need mock mode call `makeMockReadingFlowCoordinator()` directly).
    /// Production callers should not pass `useReal`.
    public static func makeDefaultReadingFlowCoordinator(useReal: Bool = false) -> ReadingFlowCoordinator {
        if useReal {
            // Legacy escape hatch — only tests preserving the old Swift Core
            // factory path should use this. Production never sets useReal=true.
            return makeRealReadingFlowCoordinator()
        }
        // S6.2: Rust Core is the default. If boot fails, fall back to mock so
        // the app never silently runs without a business path. This fallback
        // is logged and will be surfaced in the next commit's evidence layer.
        if let coordinator = makeRustCoreReadingFlowCoordinator() {
            return coordinator
        }
        print("[ShellAssembly] Rust Core boot failed — falling back to mock coordinator")
        return makeMockReadingFlowCoordinator()
    }
```

- [ ] **Step 3: 标记 `makeRealReadingFlowCoordinator` 为 deprecated(不删,留 strangler fallback)**

在 `makeRealReadingFlowCoordinator` 函数声明前加 `@available(*, deprecated, message: "S6.2: use makeRustCoreReadingFlowCoordinator — old Swift Core ReaderCoreServiceFactory path will be removed in S7")`:

```swift
    @available(*, deprecated, message: "S6.2: use makeRustCoreReadingFlowCoordinator — old Swift Core ReaderCoreServiceFactory path will be removed in S7")
    public static func makeRealReadingFlowCoordinator() -> ReadingFlowCoordinator {
```

- [ ] **Step 4: 重命名 `MockSearchService/MockTOCService/MockContentService` 为 `ProviderBacked*Service`(消除误导命名)**

这三个类在 ShellAssembly.swift line 127-212,实际是 thin delegate 到 `ReaderCoreServiceProvider`,在 rustCore 模式下会路由到 RustCore*Service。重命名以反映真实行为。

用 Edit 工具做三次 replace_all:

`MockSearchService` → `ProviderBackedSearchService`
`MockTOCService` → `ProviderBackedTOCService`
`MockContentService` → `ProviderBackedContentService`

同时更新 `makeMockReadingFlowCoordinator` 内的引用:
```swift
    public static func makeMockReadingFlowCoordinator() -> ReadingFlowCoordinator {
        let serviceProvider = ReaderCoreServiceProvider.shared

        let coordinator = ReadingFlowCoordinator(
            bookSourceRepository: InMemoryBookSourceRepository(),
            bookSourceDecoder: DefaultBookSourceDecoder(),
            searchService: ProviderBackedSearchService(provider: serviceProvider),
            tocService: ProviderBackedTOCService(provider: serviceProvider),
            contentService: ProviderBackedContentService(provider: serviceProvider),
            errorLogger: InMemoryErrorLogger()
        )

        if let searchService = coordinator.searchService as? ProviderBackedSearchService {
            searchService.onWarning = { [weak coordinator] warning in
                coordinator?.lastWarning = warning
            }
        }
        if let tocService = coordinator.tocService as? ProviderBackedTOCService {
            tocService.onWarning = { [weak coordinator] warning in
                coordinator?.lastWarning = warning
            }
        }
        if let contentService = coordinator.contentService as? ProviderBackedContentService {
            contentService.onWarning = { [weak coordinator] warning in
                coordinator?.lastWarning = warning
            }
        }

        return coordinator
    }
```

- [ ] **Step 5: 验证编译**

```bash
cd "/Users/minliny/Documents/Reader for iOS"
xcodebuild build -project ReaderForIOS.xcodeproj -scheme ReaderForIOSApp \
  -configuration Debug -destination 'platform=iOS Simulator,name=iPhone 17 Pro' 2>&1 | tail -20
```
Expected: BUILD SUCCEEDED。若失败,常见原因:
- `MockSearchService` 引用未全部替换 → grep 检查 `grep -rn "MockSearchService\|MockTOCService\|MockContentService" iOS/`
- 测试文件引用了旧名 → 在 Task 5 修复

- [ ] **Step 6: 跑 shell smoke 确认基线不退**

```bash
bash iOS/ReaderCoreNativeAdapter/run-shell-smoke.sh 2>&1 | tail -5
```
Expected: `[core] pass=29 fail=0`、`[app-side] pass=4 fail=0`(或 33/33 PASS)。

- [ ] **Step 7: 提交**

```bash
cd "/Users/minliny/Documents/Reader for iOS"
git add iOS/Shell/ShellAssembly.swift
git commit -m "refactor(ios/s6.2): make Rust Core the explicit default in ShellAssembly

- makeDefaultReadingFlowCoordinator now calls makeRustCoreReadingFlowCoordinator
  first, falling back to mock only if Rust Core boot fails (logged).
- makeRealReadingFlowCoordinator (old Swift Core ReaderCoreServiceFactory path)
  marked @available(*, deprecated) — retained as strangler fallback for S7 removal.
- Renamed MockSearchService/MockTOCService/MockContentService →
  ProviderBackedSearchService/TOCService/ContentService to reflect that they
  delegate to ReaderCoreServiceProvider (which routes to RustCore*Service when
  mode == .rustCore). The old name was misleading: these are not mocks, they
  are thin provider delegates.

Per charter §10.5: strangler pattern — old path retained as deprecated fallback,
not deleted. Business path now explicitly sources from Rust Core via C ABI."
```

---

### Task 2: ReaderApp.swift 显式 Rust Core 启动,移除 useRealServices 开关

**Files:**
- Modify: `/Users/minliny/Documents/Reader for iOS/iOS/App/ReaderApp.swift`

**背景**:`ReaderApp.swift` line 18 有 `@AppStorage("useRealServices") private var useRealServices = false`,line 28-29 读该 flag 决定 `makeDefaultReadingFlowCoordinator(useReal: useReal)`。S6.2 后 Rust Core 是默认,这个开关误导用户以为有"非 real"模式。移除它。

- [ ] **Step 1: 读 ReaderApp.swift line 14-30 确认当前状态**

```bash
cd "/Users/minliny/Documents/Reader for iOS"
sed -n '14,30p' iOS/App/ReaderApp.swift
```
Expected: 看到 `@AppStorage("useRealServices")` 和 `let useReal = UserDefaults.standard.bool(forKey: "useRealServices")`。

- [ ] **Step 2: 移除 useRealServices 开关,显式调 makeDefaultReadingFlowCoordinator()**

用 Edit 工具替换:

旧代码(line 18):
```swift
    @AppStorage("useRealServices") private var useRealServices = false
```
替换为空(删除该行)。

旧代码(line 27-30):
```swift
    public init() {
        let useReal = UserDefaults.standard.bool(forKey: "useRealServices")
        let coordinator = ShellAssembly.makeDefaultReadingFlowCoordinator(useReal: useReal)
        _coordinator = StateObject(wrappedValue: coordinator)
```
替换为:
```swift
    public init() {
        // S6.2: Rust Core is the default business path. The legacy
        // useRealServices UserDefaults toggle is removed — production never
        // needs the old Swift Core factory path. Tests that need mock mode
        // inject it via ShellAssembly.makeMockReadingFlowCoordinator().
        let coordinator = ShellAssembly.makeDefaultReadingFlowCoordinator()
        _coordinator = StateObject(wrappedValue: coordinator)
```

- [ ] **Step 3: 保留 S6.1 的 Rust Core boot 逻辑(line 40-48 不动)**

确认 line 40-48 的 `RustCoreRuntimeHolder.shared.boot()` + `configureRustCoreMode()` 仍在 —— 这是 provider 模式切换的关键。

- [ ] **Step 4: 验证编译**

```bash
cd "/Users/minliny/Documents/Reader for iOS"
xcodebuild build -project ReaderForIOS.xcodeproj -scheme ReaderForIOSApp \
  -configuration Debug -destination 'platform=iOS Simulator,name=iPhone 17 Pro' 2>&1 | tail -10
```
Expected: BUILD SUCCEEDED。

- [ ] **Step 5: 提交**

```bash
cd "/Users/minliny/Documents/Reader for iOS"
git add iOS/App/ReaderApp.swift
git commit -m "refactor(ios/s6.2): remove useRealServices toggle from ReaderApp

The @AppStorage('useRealServices') UserDefaults flag and makeDefault(useReal:)
call are removed — Rust Core is now the unconditional default per Task 1.
S6.1 Rust Core boot (RustCoreRuntimeHolder.shared.boot() + configureRustCoreMode())
is retained as the provider mode switch.

Tests needing mock mode inject it directly via
ShellAssembly.makeMockReadingFlowCoordinator(), not via UserDefaults."
```

---

### Task 3: ReaderCoreServiceProvider 默认模式切到 .rustCore

**Files:**
- Modify: `/Users/minliny/Documents/Reader for iOS/iOS/CoreBridge/ReaderCoreServiceProvider.swift`

**背景**:`ReaderCoreServiceProvider` line 22 `private var mode: ServiceMode = .mock`。虽然 `ReaderApp.swift` 启动时调 `configureRustCoreMode()` 会把 mode 切到 `.rustCore`,但默认值仍是 `.mock`,意味着任何未走 `ReaderApp.init` 的入口(如测试、preview)都会落到 mock。把默认值改成 `.rustCore` 让"不配置就是 Rust Core"。

- [ ] **Step 1: 修改默认 mode 为 .rustCore**

用 Edit 工具替换 `iOS/CoreBridge/ReaderCoreServiceProvider.swift` line 22:

旧:
```swift
    private var mode: ServiceMode = .mock
```
新:
```swift
    // S6.2: Default mode is .rustCore. ReaderApp.init() still calls
    // configureRustCoreMode() to boot the runtime + wire RustCore*Service
    // adapters; this default ensures tests/previews that skip ReaderApp.init
    // also route through Rust Core (or fail loudly if runtime not booted,
    // rather than silently falling back to mock data).
    private var mode: ServiceMode = .rustCore
```

- [ ] **Step 2: 在 searchBooks/getChapterList/getChapterContent 的 rustCore 分支加 fallback 诊断**

当前 line 208-234(searchBooks)在 `mode == .rustCore` 但 `rustCoreSearchService == nil` 时会落到下面的 `canUseRealService`/`controlledOnline`/`mock` 分支。加诊断日志让"runtime 未 boot"可见。

用 Edit 工具替换 searchBooks 的 rustCore 分支(line 209-220):

旧:
```swift
        #if canImport(ReaderCoreNativeAdapter)
        if mode == .rustCore, let service = rustCoreSearchService, let source {
            do {
                let results = try await service.search(source: source, query: SearchQuery(keyword: keyword, page: page))
                return results.isEmpty ? .empty : .loaded(results)
            } catch let error as AppReaderError {
                return .failed(error)
            } catch {
                return .failed(AppReaderError(code: .unknown, message: error.localizedDescription, stage: "SEARCH"))
            }
        }
        #endif
```
新:
```swift
        #if canImport(ReaderCoreNativeAdapter)
        if mode == .rustCore {
            guard let service = rustCoreSearchService else {
                return .failed(AppReaderError(code: .unknown, message: "[RustCore] search service not configured — runtime not booted?", stage: "SEARCH"))
            }
            guard let source else {
                return .failed(AppReaderError(code: .unsupported, message: "No book source selected for Rust Core search", stage: "SEARCH"))
            }
            do {
                let results = try await service.search(source: source, query: SearchQuery(keyword: keyword, page: page))
                return results.isEmpty ? .empty : .loaded(results)
            } catch let error as AppReaderError {
                return .failed(error)
            } catch {
                return .failed(AppReaderError(code: .unknown, message: error.localizedDescription, stage: "SEARCH"))
            }
        }
        #endif
```

对 `getChapterList`(line 329-341)和 `getChapterContent`(line 385-397)做同样改造:把 `if mode == .rustCore, let service = rustCore*, let source` 拆成 `if mode == .rustCore { guard let service ... guard let source ... }`。

- [ ] **Step 3: 验证编译**

```bash
cd "/Users/minliny/Documents/Reader for iOS"
xcodebuild build -project ReaderForIOS.xcodeproj -scheme ReaderForIOSApp \
  -configuration Debug -destination 'platform=iOS Simulator,name=iPhone 17 Pro' 2>&1 | tail -10
```
Expected: BUILD SUCCEEDED。

- [ ] **Step 4: 跑 shell smoke 确认基线不退**

```bash
bash iOS/ReaderCoreNativeAdapter/run-shell-smoke.sh 2>&1 | tail -5
```
Expected: 33/33 PASS。

- [ ] **Step 5: 提交**

```bash
cd "/Users/minliny/Documents/Reader for iOS"
git add iOS/CoreBridge/ReaderCoreServiceProvider.swift
git commit -m "refactor(ios/s6.2): default ServiceMode to .rustCore + fail-loud on missing runtime

- mode default changed from .mock to .rustCore — tests/previews that skip
  ReaderApp.init() now route through Rust Core (or fail loudly if runtime
  not booted) instead of silently returning mock data.
- searchBooks/getChapterList/getChapterContent: rustCore branch now uses
  guard-let to surface 'runtime not booted' and 'no source selected' as
  explicit errors instead of falling through to mock/real/controlledOnline.

Per charter §10.3: fail-loud > silent fallback — silent mock fallback masks
Rust Core wiring bugs."
```

---

### Task 4: 新建 RustCoreBookDetailService 补 book.detail 缺口

**Files:**
- Create: `/Users/minliny/Documents/Reader for iOS/iOS/CoreBridge/RustCoreBookDetailService.swift`
- Modify: `/Users/minliny/Documents/Reader for iOS/iOS/CoreBridge/ReaderCoreServiceProvider.swift`

**背景**:`ReaderCoreServiceProvider.getBookDetail`(line 294-313)当前在 rustCore 模式下**没有 Rust Core 分支** —— 它只检查 `canUseRealService`(旧 Swift Core)和 `controlledOnline`/`offlineReplay`/`mock`。这意味着 Rust Core 模式下 book detail 会落到 mock,返回 `SearchResultItem(title: source.bookSourceName, ...)` 这种空壳。需要补 `RustCoreBookDetailService` dispatch `book.detail` 到 Rust Core。

- [ ] **Step 1: 创建 RustCoreBookDetailService.swift**

`/Users/minliny/Documents/Reader for iOS/iOS/CoreBridge/RustCoreBookDetailService.swift`:

```swift
// CoreBridge
//
// RustCoreBookDetailService: dispatches `book.detail` to Rust Core via C ABI.
// Core auto-builds the detail request from the source's `ruleBookInfo` +
// the book's bookUrl, emits `host.request` (http.execute), the HostRequestRouter
// executes it via URLSessionHTTPClient, Core parses the response using Legado
// DSL (ruleBookInfo), and returns the enriched book metadata.
//
// S6.2: This closes the gap where ReaderCoreServiceProvider.getBookDetail had
// no rustCore branch and fell through to mock (returning a title-only shell).

import Foundation
import ReaderCoreModels
import ReaderCoreProtocols
import ReaderCoreNativeAdapter

public final class RustCoreBookDetailService: @unchecked Sendable {
    private let runtime: ReaderCoreNativeRuntime
    private let router: HostRequestRouter
    private let requestTimeout: TimeInterval

    public init(
        runtime: ReaderCoreNativeRuntime,
        router: HostRequestRouter? = nil,
        requestTimeout: TimeInterval = 15
    ) {
        self.runtime = runtime
        self.router = router ?? RustCoreServiceSupport.makeRouter(runtime: runtime)
        self.requestTimeout = requestTimeout
    }

    /// Fetch book detail (enriched metadata) via Rust Core `book.detail`.
    /// - Parameters:
    ///   - source: The BookSource providing ruleBookInfo.
    ///   - book: The SearchResultItem from search (must have detailURL == bookUrl).
    /// - Returns: Enriched SearchResultItem with intro/coverUrl/author/etc.
    public func fetchDetail(source: BookSource, book: SearchResultItem) async throws -> SearchResultItem {
        let sourceId = source.id?.isEmpty == false ? source.id! : UUID().uuidString
        let inlineSource = RustCoreServiceSupport.serializeSource(source)

        let params: [String: Any] = [
            "sourceId": sourceId,
            "book": [
                "bookUrl": book.detailURL,
                "title": book.title,
                "author": book.author ?? "",
                "coverUrl": book.coverURL ?? "",
                "intro": book.intro ?? "",
            ],
            "source": inlineSource,
        ]
        let requestId: UInt64 = UInt64(Date().timeIntervalSince1970 * 1000) % 1_000_000 + 200_000
        let command: [String: Any] = [
            "protocolVersion": 1,
            "requestId": NSNumber(value: requestId),
            "method": "book.detail",
            "params": params,
        ]

        do {
            let json = try JSONSerialization.data(withJSONObject: command)
            try runtime.send(json: json)

            // Expect host.request (http.execute) from Core for ruleBookInfo URL.
            let hostRequest = try RustCoreServiceSupport.pollEvent(
                runtime: runtime, requestId: requestId, timeout: requestTimeout
            )
            if hostRequest.type == "error" {
                throw ReaderCoreNativeError.coreError(
                    code: hostRequest.coreErrorCode ?? "INTERNAL",
                    message: hostRequest.coreErrorMessage ?? "book.detail failed"
                )
            }
            guard hostRequest.type == "host.request" else {
                throw ReaderCoreNativeError.coreError(
                    code: "INTERNAL",
                    message: "expected host.request, got \(hostRequest.type)"
                )
            }
            try await router.handleHostRequest(hostRequest)

            // Expect result (with enriched book) or error.
            let result = try RustCoreServiceSupport.pollEvent(
                runtime: runtime, requestId: requestId, timeout: requestTimeout
            )
            if result.type == "error" {
                throw ReaderCoreNativeError.coreError(
                    code: result.coreErrorCode ?? "INTERNAL",
                    message: result.coreErrorMessage ?? "book.detail result failed"
                )
            }
            guard result.type == "result" else {
                throw ReaderCoreNativeError.coreError(
                    code: "INTERNAL",
                    message: "expected result, got \(result.type)"
                )
            }
            return Self.parseBookDetail(result.data, fallback: book)
        } catch let error as ReaderCoreNativeError {
            throw RustCoreServiceSupport.mapCoreError(error)
        } catch {
            throw RustCoreServiceSupport.mapCoreError(error)
        }
    }

    /// Parse Core `result.data.book` → enriched `SearchResultItem`.
    /// Falls back to the original `book` for fields Core didn't return.
    private static func parseBookDetail(_ data: [String: Any]?, fallback: SearchResultItem) -> SearchResultItem {
        guard let book = data?["book"] as? [String: Any] else {
            return fallback
        }
        return SearchResultItem(
            title: (book["title"] as? String) ?? fallback.title,
            detailURL: (book["bookUrl"] as? String) ?? fallback.detailURL,
            author: (book["author"] as? String) ?? fallback.author,
            coverURL: (book["coverUrl"] as? String) ?? fallback.coverURL,
            intro: (book["intro"] as? String) ?? fallback.intro
        )
    }
}
```

- [ ] **Step 2: 在 ReaderCoreServiceProvider 加 rustCoreBookDetailService 字段 + getBookDetail rustCore 分支**

用 Edit 工具在 `ReaderCoreServiceProvider.swift` 的 `#if canImport(ReaderCoreNativeAdapter)` 字段块(line 32-36)后加:

```swift
    #if canImport(ReaderCoreNativeAdapter)
    private var rustCoreSearchService: (any SearchService)?
    private var rustCoreTOCService: (any TOCService)?
    private var rustCoreContentService: (any ContentService)?
    private var rustCoreBookDetailService: RustCoreBookDetailService?
    #endif
```

在 `configureRustCoreMode()`(line 146-165)的 lock 块内加:

```swift
        lock.lock()
        rustCoreSearchService = RustCoreSearchService(runtime: runtime)
        rustCoreTOCService = RustCoreTOCService(runtime: runtime)
        rustCoreContentService = RustCoreContentService(runtime: runtime)
        rustCoreBookDetailService = RustCoreBookDetailService(runtime: runtime)
        mode = .rustCore
        lock.unlock()
        return rustCoreSearchService != nil
```

修改 `getBookDetail`(line 294-313)在开头加 rustCore 分支:

```swift
    public func getBookDetail(bookURL: String, source: BookSource? = nil) async -> LoadState<SearchResultItem> {
        #if canImport(ReaderCoreNativeAdapter)
        if mode == .rustCore {
            guard let service = rustCoreBookDetailService else {
                return .failed(AppReaderError(code: .unknown, message: "[RustCore] book detail service not configured — runtime not booted?", stage: "DETAIL"))
            }
            guard let source else {
                return .failed(AppReaderError(code: .unsupported, message: "No book source selected for Rust Core detail", stage: "DETAIL"))
            }
            let inputBook = SearchResultItem(
                title: source.bookSourceName,
                detailURL: bookURL,
                author: nil,
                coverURL: nil,
                intro: nil
            )
            do {
                let enriched = try await service.fetchDetail(source: source, book: inputBook)
                return .loaded(enriched)
            } catch let error as AppReaderError {
                return .failed(error)
            } catch {
                return .failed(AppReaderError(code: .unknown, message: error.localizedDescription, stage: "DETAIL"))
            }
        }
        #endif
        if canUseRealService, let source {
            return await performRealBookDetail(bookURL: bookURL, source: source)
        }
        // ... 其余 controlledOnline/OfflineReplay/mock 分支不动
```

- [ ] **Step 3: 验证编译**

```bash
cd "/Users/minliny/Documents/Reader for iOS"
xcodebuild build -project ReaderForIOS.xcodeproj -scheme ReaderForIOSApp \
  -configuration Debug -destination 'platform=iOS Simulator,name=iPhone 17 Pro' 2>&1 | tail -10
```
Expected: BUILD SUCCEEDED。

- [ ] **Step 4: 提交**

```bash
cd "/Users/minliny/Documents/Reader for iOS"
git add iOS/CoreBridge/RustCoreBookDetailService.swift iOS/CoreBridge/ReaderCoreServiceProvider.swift
git commit -m "feat(ios/s6.2): add RustCoreBookDetailService + wire book.detail via Rust Core

- New RustCoreBookDetailService dispatches book.detail to Rust Core, parsing
  result.data.book into enriched SearchResultItem (title/author/coverUrl/intro).
- ReaderCoreServiceProvider.getBookDetail now has a rustCore branch (previously
  fell through to mock, returning a title-only shell).
- configureRustCoreMode() instantiates rustCoreBookDetailService alongside
  search/toc/content.

Closes the S6.1 gap where book detail was the only reading-flow step not
dispatched through Rust Core. Now search→detail→toc→content all route via
Rust Core C ABI when mode == .rustCore."
```

---

### Task 5: 退役 Default{Search,TOC,Content}Service 从生产路径(保留为 deprecated)

**Files:**
- Modify: `/Users/minliny/Documents/Reader for iOS/iOS/CoreIntegration/DefaultSearchService.swift`
- Modify: `/Users/minliny/Documents/Reader for iOS/iOS/CoreIntegration/DefaultTOCService.swift`
- Modify: `/Users/minliny/Documents/Reader for iOS/iOS/CoreIntegration/DefaultContentService.swift`

**背景**:这三个文件是旧 Swift Core `ReaderCoreServiceFactory` 的包装层,S6.2 后 `makeRealReadingFlowCoordinator` 已 deprecated,这三个 wrapper 也应标记 deprecated。不删除 —— strangler 模式保留 fallback。

- [ ] **Step 1: 在每个 Default*Service 类声明前加 @available deprecated**

用 Edit 工具对三个文件分别加:

`iOS/CoreIntegration/DefaultSearchService.swift`:
```swift
@available(*, deprecated, message: "S6.2: use RustCoreSearchService via ShellAssembly.makeDefaultReadingFlowCoordinator — old Swift Core factory path will be removed in S7")
public final class DefaultSearchService: SearchService {
```

`iOS/CoreIntegration/DefaultTOCService.swift`:
```swift
@available(*, deprecated, message: "S6.2: use RustCoreTOCService via ShellAssembly.makeDefaultReadingFlowCoordinator — old Swift Core factory path will be removed in S7")
public final class DefaultTOCService: TOCService {
```

`iOS/CoreIntegration/DefaultContentService.swift`:
```swift
@available(*, deprecated, message: "S6.2: use RustCoreContentService via ShellAssembly.makeDefaultReadingFlowCoordinator — old Swift Core factory path will be removed in S7")
public final class DefaultContentService: ContentService {
```

- [ ] **Step 2: 确认 ShellAssembly.makeRealReadingFlowCoordinator 是唯一引用方**

```bash
cd "/Users/minliny/Documents/Reader for iOS"
grep -rn "DefaultSearchService\|DefaultTOCService\|DefaultContentService" iOS/ --include="*.swift" | grep -v "Tests/" | grep -v "// "
```
Expected: 只剩 `Shell/ShellAssembly.swift` 的 `makeRealReadingFlowCoordinator`(已 deprecated)和三个文件自身的定义。若有其他生产引用,需先迁移到 RustCore*Service。

- [ ] **Step 3: 验证编译(deprecation warning 应只出现在 makeRealReadingFlowCoordinator)**

```bash
xcodebuild build -project ReaderForIOS.xcodeproj -scheme ReaderForIOSApp \
  -configuration Debug -destination 'platform=iOS Simulator,name=iPhone 17 Pro' 2>&1 | grep -E "deprecat|error" | head -10
```
Expected: 可能有 deprecation warning 来自 `makeRealReadingFlowCoordinator`,无 error。

- [ ] **Step 4: 提交**

```bash
cd "/Users/minliny/Documents/Reader for iOS"
git add iOS/CoreIntegration/DefaultSearchService.swift \
        iOS/CoreIntegration/DefaultTOCService.swift \
        iOS/CoreIntegration/DefaultContentService.swift
git commit -m "refactor(ios/s6.2): mark Default{Search,TOC,Content}Service as deprecated

These three wrappers around the old Swift Core ReaderCoreServiceFactory are
marked @available(*, deprecated). The only remaining production reference is
ShellAssembly.makeRealReadingFlowCoordinator (itself deprecated in Task 1).
RustCore*Service via ShellAssembly.makeDefaultReadingFlowCoordinator is the
non-deprecated business path.

Per charter §10.5: strangler pattern — deprecated, not deleted. S7 will remove
them once Rust Core E2E is proven on real device."
```

---

### Task 6: 修复测试中 MockSearchService 旧名引用

**Files:**
- Modify: 任何引用 `MockSearchService/MockTOCService/MockContentService` 的测试文件(Task 1 Step 4 重命名后)

- [ ] **Step 1: 找出所有引用旧名的测试文件**

```bash
cd "/Users/minliny/Documents/Reader for iOS"
grep -rn "MockSearchService\|MockTOCService\|MockContentService" iOS/Tests/ iOS/ShellSmokeTests/ iOS/ReaderCoreNativeAdapter/ShellSmokeTests/ 2>/dev/null
```
Expected: 列出所有引用旧名的测试文件。

- [ ] **Step 2: 对每个测试文件做 replace_all 重命名**

对每个文件用 Edit 工具 replace_all:
- `MockSearchService` → `ProviderBackedSearchService`
- `MockTOCService` → `ProviderBackedTOCService`
- `MockContentService` → `ProviderBackedContentService`

**注意**:`MockReaderCoreService`(真正的 mock 数据生成器)**不重命名** —— 它仍然提供 mock 数据,测试可注入。

- [ ] **Step 3: 验证测试编译**

```bash
cd "/Users/minliny/Documents/Reader for iOS"
xcodebuild build -project ReaderForIOS.xcodeproj -scheme ReaderForIOSApp \
  -configuration Debug -destination 'platform=iOS Simulator,name=iPhone 17 Pro' 2>&1 | grep -E "error" | head -10
```
Expected: 无 error。

- [ ] **Step 4: 跑 shell smoke + iOS-sim XCTest 确认基线**

```bash
bash iOS/ReaderCoreNativeAdapter/run-shell-smoke.sh 2>&1 | tail -3
xcodebuild -scheme ReaderCoreNativeAdapterSmokeTests \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro' test 2>&1 | tail -5
```
Expected: shell smoke 33/33 PASS;iOS-sim XCTest 9/9 PASS(或同等基线)。

- [ ] **Step 5: 提交**

```bash
cd "/Users/minliny/Documents/Reader for iOS"
git add iOS/Tests/ iOS/ShellSmokeTests/ iOS/ReaderCoreNativeAdapter/ShellSmokeTests/
git commit -m "test(ios/s6.2): rename MockSearchService refs to ProviderBacked*Service in tests

Follows Task 1 Step 4 rename. MockReaderCoreService (the actual mock data
generator) is NOT renamed — it remains available for test injection."
```

---

### Task 7: 新建 RustCoreEndToEndTest 验证 search→detail→toc→content via Rust Core

**Files:**
- Create: `/Users/minliny/Documents/Reader for iOS/iOS/Tests/ReaderAppTests/RustCoreEndToEndTest.swift`

**背景**:S6.1 的证明停在 shell smoke(wrapper smoke 级别)。S6.2 需要 App 级 E2E 证明:在 iOS 模拟器上,通过 Rust Core C ABI 跑通 `search→detail→toc→content`,且**不经过 mock**(断言结果非 mock 数据)。

- [ ] **Step 1: 检查现有 test fixture 书源**

```bash
cd "/Users/minliny/Documents/Reader for iOS"
ls iOS/Tests/Fixtures/BookSources/ iOS/AppSupport/Sources/ 2>/dev/null | grep -i "json\|source"
cat iOS/AppSupport/Sources/sample_book_source.json 2>/dev/null | head -30
```
Expected: 看到现有书源 fixture(如 `sample_book_source.json`、`xingxingxsw.search-only.json`)。E2E 测试需要一个含 searchUrl + ruleSearch + ruleBookInfo + ruleToc + ruleContent 的完整书源。

- [ ] **Step 2: 创建 RustCoreEndToEndTest.swift**

`/Users/minliny/Documents/Reader for iOS/iOS/Tests/ReaderAppTests/RustCoreEndToEndTest.swift`:

```swift
// Tests/ReaderAppTests
//
// RustCoreEndToEndTest: proves search→detail→toc→content via Rust Core C ABI
// on iOS Simulator. This is App/simulator-level proof (charter §10.3), NOT
// real device proof (deferred to S7).
//
// S6.2: This test is the closure evidence for "iOS business path switched to
// Rust Core". It MUST:
// 1. Boot Rust Core runtime (not mock)
// 2. Dispatch book.search via RustCoreSearchService → assert non-mock results
// 3. Dispatch book.detail via RustCoreBookDetailService → assert enriched
// 4. Dispatch book.toc via RustCoreTOCService → assert non-empty chapters
// 5. Dispatch chapter.content via RustCoreContentService → assert non-empty
//
// Mock-only assertions (MockReaderCoreService data) do NOT satisfy this test.

import XCTest
@testable import ReaderApp
import ReaderCoreModels
import ReaderCoreProtocols
import ReaderCoreNativeAdapter

final class RustCoreEndToEndTest: XCTestCase {

    private var runtime: ReaderCoreNativeRuntime!
    private var searchService: RustCoreSearchService!
    private var detailService: RustCoreBookDetailService!
    private var tocService: RustCoreTOCService!
    private var contentService: RustCoreContentService!

    /// Test book source: a minimal Legado-format source with real rules.
    /// NOTE: This test hits the real network (host.request → URLSessionHTTPClient
    /// → real HTTP). It is NOT a unit test. Run on simulator with network.
    private let testBookSourceJSON = """
    {
      "bookSourceName": "S6.2 E2E Test Source",
      "bookSourceUrl": "https://www.biquges123.com",
      "bookSourceType": 0,
      "enabled": true,
      "searchUrl": "https://www.biquges123.com/search.php?q={{key}}",
      "ruleSearch": {
        "bookList": "css:.result-list .item",
        "name": "css:.book-name@text",
        "author": "css:.book-author@text",
        "bookUrl": "css:a@href"
      },
      "ruleBookInfo": {
        "name": "css:h1@text",
        "author": "css:.author@text",
        "intro": "css:.intro@text"
      },
      "ruleToc": {
        "chapterList": "css:.chapter-list a",
        "chapterName": "@text",
        "chapterUrl": "@href"
      },
      "ruleContent": {
        "content": "css:.content@html"
      }
    }
    """

    override func setUpWithError() throws {
        try super.setUpWithError()
        // Boot Rust Core runtime — NOT mock.
        try RustCoreRuntimeHolder.shared.boot()
        runtime = try XCTUnwrap(RustCoreRuntimeHolder.shared.current, "Rust Core runtime must boot")
        searchService = RustCoreSearchService(runtime: runtime)
        detailService = RustCoreBookDetailService(runtime: runtime)
        tocService = RustCoreTOCService(runtime: runtime)
        contentService = RustCoreContentService(runtime: runtime)
    }

    override func tearDownWithError() throws {
        runtime = nil
        searchService = nil
        detailService = nil
        tocService = nil
        contentService = nil
        try super.tearDownWithError()
    }

    func testSearchViaRustCoreReturnsNonMockResults() async throws {
        try XCTSkipIf(ProcessInfo.processInfo.environment["SKIP_NETWORK_TESTS"] != nil,
                      "SKIP_NETWORK_TESTS set — skipping E2E network test")

        let source = try JSONDecoder().decode(BookSource.self, from: testBookSourceJSON.data(using: .utf8)!)
        let results = try await searchService.search(
            source: source,
            query: SearchQuery(keyword: "凡人修仙传", page: 1)
        )

        XCTAssertFalse(results.isEmpty, "Rust Core search must return non-empty results — if empty, check network or source rules")
        let first = try XCTUnwrap(results.first)
        XCTAssertFalse(first.title.isEmpty, "Search result title must be non-empty")
        XCTAssertFalse(first.detailURL.isEmpty, "Search result detailURL must be non-empty (needed for detail/toc)")

        // Anti-mock assertion: mock data has title "Mock Book N" — fail if seen.
        XCTAssertFalse(first.title.hasPrefix("Mock "), "Result looks like mock data: \(first.title)")
    }

    func testSearchDetailTocContentViaRustCore() async throws {
        try XCTSkipIf(ProcessInfo.processInfo.environment["SKIP_NETWORK_TESTS"] != nil,
                      "SKIP_NETWORK_TESTS set — skipping E2E network test")

        let source = try JSONDecoder().decode(BookSource.self, from: testBookSourceJSON.data(using: .utf8)!)

        // 1. Search
        let results = try await searchService.search(
            source: source,
            query: SearchQuery(keyword: "凡人修仙传", page: 1)
        )
        XCTAssertFalse(results.isEmpty, "Search must return results")
        let firstBook = try XCTUnwrap(results.first)

        // 2. Detail
        let enriched = try await detailService.fetchDetail(source: source, book: firstBook)
        XCTAssertFalse(enriched.title.isEmpty, "Detail must return non-empty title")
        // Anti-mock: mock detail returns title = source.bookSourceName
        XCTAssertNotEqual(enriched.title, source.bookSourceName,
                          "Detail title equals source name — looks like mock fallback")

        // 3. TOC
        let chapters = try await tocService.fetchTOC(source: source, detailURL: firstBook.detailURL)
        XCTAssertFalse(chapters.isEmpty, "TOC must return non-empty chapters")
        let firstChapter = try XCTUnwrap(chapters.first)
        XCTAssertFalse(firstChapter.chapterURL.isEmpty, "Chapter URL must be non-empty (needed for content)")

        // 4. Content
        let page = try await contentService.fetchContent(source: source, chapterURL: firstChapter.chapterURL)
        XCTAssertFalse(page.content.isEmpty, "Content must be non-empty")
        // Anti-mock: mock content has "Mock content for chapter"
        XCTAssertFalse(page.content.contains("Mock content"), "Content looks like mock: \(page.content.prefix(50))")
    }
}
```

- [ ] **Step 3: 把测试加到 ReaderAppTests target**

检查 `iOS/Package.swift` 或 `project.yml` 确认 `ReaderAppTests` target 自动包含 `Tests/ReaderAppTests/` 目录下所有 .swift 文件。若用 project.yml,通常 `sources` 是目录通配,无需手动加。

```bash
cd "/Users/minliny/Documents/Reader for iOS"
grep -A 5 "ReaderAppTests" project.yml 2>/dev/null || grep -A 5 "ReaderAppTests" iOS/Package.swift
```
Expected: 看到 ReaderAppTests target 定义,sources 指向目录(自动包含新文件)。

- [ ] **Step 4: 编译测试**

```bash
xcodebuild build-for-testing -project ReaderForIOS.xcodeproj -scheme ReaderForIOSApp \
  -configuration Debug -destination 'platform=iOS Simulator,name=iPhone 17 Pro' 2>&1 | tail -10
```
Expected: BUILD SUCCEEDED。若失败,常见原因:
- `@testable import ReaderApp` 失败 → 检查 target 名是否匹配
- `RustCoreBookDetailService` 未在 target 中 → 确认 CoreBridge 目录在 ReaderApp target sources 中

- [ ] **Step 5: 跑 E2E 测试(需模拟器启动 + 网络)**

```bash
# 确保有 booted simulator
xcrun simctl list devices booted | grep "iPhone 17 Pro" || \
  xcrun simctl boot "iPhone 17 Pro" 2>/dev/null; sleep 5

xcodebuild test-without-building -project ReaderForIOS.xcodeproj -scheme ReaderForIOSApp \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro' \
  -only-testing:ReaderAppTests/RustCoreEndToEndTest 2>&1 | tail -20
```
Expected: 2 tests pass(或 skip if `SKIP_NETWORK_TESTS` set)。若失败:
- `runtime must boot` → Rust Core runtime 创建失败,检查 xcframework 是否就位
- `Search must return non-empty` → 网络问题或书源规则失效,换书源或检查 Core conformance
- `looks like mock data` → 业务路径未真正切到 Rust Core,检查 provider mode

- [ ] **Step 6: 提交**

```bash
cd "/Users/minliny/Documents/Reader for iOS"
git add iOS/Tests/ReaderAppTests/RustCoreEndToEndTest.swift
git commit -m "test(ios/s6.2): add RustCoreEndToEndTest — search→detail→toc→content via Rust Core

App/simulator-level proof (charter §10.3) that the iOS business path routes
through Rust Core C ABI, NOT mock:

1. testSearchViaRustCoreReturnsNonMockResults: book.search via RustCoreSearchService,
   asserts non-empty results + anti-mock assertion (title must not start with 'Mock ').
2. testSearchDetailTocContentViaRustCore: full reading flow
   search→detail→toc→content, each step via Rust Core service, with anti-mock
   assertions (detail title != source name, content must not contain 'Mock content').

Tests are skipped when SKIP_NETWORK_TESTS env var is set (offline CI fallback).
Real device proof deferred to S7.

Evidence layering:
- shell smoke (33/33) = wrapper smoke
- ReaderCoreNativeAdapterSmokeTests (9/9) = iOS-sim wrapper smoke
- RustCoreEndToEndTest (this) = App/simulator proof, NOT real device
- Real device proof = S7"
```

---

### Task 8: 章程 §9 五问 + 最终状态摘要

**Files:**
- No file changes — commit message body 含五问答案

- [ ] **Step 1: 整理章程 §9 五问答案**

```markdown
## 章程 §9 五问答案(S6.2/S6.3 iOS Rust Core 为默认业务路径)

1. **本轮兼容目标来自本地 legado 的哪个代码路径或数据结构**:
   - `app/src/main/java/io/legado/app/model/webBook/WebBook.kt` 四段调度(searchBookAwait/getBookInfoAwait/getChapterListAwait/getContentAwait)→ iOS RustCore{Search,BookDetail,TOC,Content}Service
   - `app/src/main/java/io/legado/app/data/entities/BookSource.kt` BookSource 模型 → 通过 `ReaderCoreModels.BookSource` 消费(Swift 模型层保留)
   - `app/src/main/java/io/legado/app/ui/book/search/SearchViewModel.kt` + `ui/book/info/BookInfoViewModel.kt` + `ui/book/read/ReadBookViewModel.kt` 状态机 → 35 个 SwiftUI 页面不动,ViewModel 调 ReaderCoreServiceProvider(provider 默认 .rustCore)

2. **本轮迁移资产来自本地 Reader-Core 的哪个代码路径**(Swift Core 已实现部分):
   - `Sources/ReaderCoreProtocols/NetworkProtocols.swift` HTTPRequest/HTTPResponse 字段定义 → Rust Core HostHttpRequest/HostHttpResponse 字段已对齐
   - `Sources/ReaderCoreParser/NonJSParserEngine.swift` 四入口 → Rust Core book.* dispatch
   - `Sources/ReaderCoreModels/BookSource.swift` Legado JSON 兼容 → Swift 模型层保留,RustCore*Service 通过 RustCoreServiceSupport.serializeSource 转换
   - 对照 Legado 新建(Reader-Core 缺失):RustCoreBookDetailService(S6.1 缺口,S6.2 补)
   - 退役:Default{Search,TOC,Content}Service(旧 Swift Core ReaderCoreServiceFactory 包装,deprecated 非删除)

3. **本轮 Rust 改动落在哪个 crate、protocol schema、C ABI 或 binding**:
   - **零 Rust 改动** — 仅消费现有 C ABI v1 + iOS binding(主仓库 bindings/ios/ + include/reader_core.h)
   - protocol schema 未改(book.search/detail/toc/chapter.content 已存在)
   - C ABI 未改(rc_runtime_create/send/cancel/destroy 已冻结 v1)
   - 预构建 ReaderCore.xcframework 已在 iOS 仓库 ReaderCoreNativeAdapter/cabi/ 就位

4. **本轮是否改变三端 host adapter 的责任边界**:
   - **否** — 仅 iOS 接入现有边界,未改 Core/Host 边界定义
   - iOS 保留(章程 §4 Host owns):URLSession HTTP transport(HostRequestRouter 用 URLSessionHTTPClient)、WebView(ProductionWebViewAdapter,本期不实现)、Keychain(WebDAVKeychainStore)、AVSpeech(ReaderTTSPlayer,本期不动)
   - Core 不开 socket、不碰 WebView、不存明文凭据(红线 4 不变)

5. **本轮证据是 crate test、CLI conformance、FFI smoke、wrapper smoke、App/device proof 还是 corpus benchmark**:
   - `bash iOS/ReaderCoreNativeAdapter/run-shell-smoke.sh` = **wrapper smoke**(33/33 PASS,macOS host)
   - `xcodebuild -scheme ReaderCoreNativeAdapterSmokeTests test` = **iOS-sim wrapper smoke**(9/9 PASS)
   - `RustCoreEndToEndTest` = **App/simulator proof**(search→detail→toc→content via Rust Core,非 mock,anti-mock 断言)
   - `xcodebuild build -scheme ReaderForIOSApp` = App 编译证明
   - **非 real device proof**(真机留 S7)
   - **非 corpus benchmark**(四端同 corpus 留 S7 主线阶段)
```

- [ ] **Step 2: 最终状态摘要**

```bash
cd "/Users/minliny/Documents/Reader for iOS"
echo "=== Branch ==="
git branch --show-current
echo "=== Commits since S6.1 (44fd48e) ==="
git log --oneline 44fd48e..HEAD
echo "=== File counts ==="
echo "Swift files total: $(find iOS -name '*.swift' | wc -l)"
echo "Files still importing ReaderCoreServices (old factory):"
grep -rl "import ReaderCoreServices" iOS/ --include="*.swift" | grep -v Tests/ | wc -l
echo "Files importing ReaderCoreModels (kept — model layer):"
grep -rl "import ReaderCoreModels" iOS/ --include="*.swift" | wc -l
echo "Files importing ReaderCoreNativeAdapter (Rust Core):"
grep -rl "import ReaderCoreNativeAdapter" iOS/ --include="*.swift" | wc -l
echo "=== Verification commands ==="
xcodebuild build -project ReaderForIOS.xcodeproj -scheme ReaderForIOSApp \
  -configuration Debug -destination 'platform=iOS Simulator,name=iPhone 17 Pro' 2>&1 | tail -3
```

- [ ] **Step 3: 提交 plan 文档到主仓库**

```bash
cd /Users/minliny/Documents/Reader-Core-Native
git add docs/superpowers/plans/2026-06-27-ios-rust-core-integration-s6.md
git commit -m "docs(plans): add iOS S6.2/S6.3 Rust Core integration plan (8 tasks)

Plan: retire old Swift Core service factory path, make Rust Core the explicit
default business path on iOS, add simulator E2E proof.

Scope:
- S6.2 (Tasks 1-6): ShellAssembly default → Rust Core, retire mock/old-factory
  from production, rename misleading Mock*Service → ProviderBacked*Service,
  add RustCoreBookDetailService to close S6.1 book.detail gap.
- S6.3 (Tasks 7-8): RustCoreEndToEndTest on simulator (search→detail→toc→content
  via Rust Core, anti-mock assertions), charter §9 five questions.

Binding readiness confirmed:
- ReaderCore.xcframework (ios-arm64-simulator + macos-arm64) prebuilt in repo
- RustCoreSearchService/TOCService/ContentService implement Swift protocols
- Shell smoke 33/33 PASS, iOS-sim XCTest 9/9 PASS (S6.1 baseline)
- ReaderApp.swift already boots Rust Core + configureRustCoreMode() at launch

Per charter §10.5: strangler pattern — old path deprecated, not deleted.
Real device proof + corpus benchmark deferred to S7."
```

---

## Self-Review

### 1. Spec coverage(用户任务 + 章程 §9 五问)

- [x] Step 1(定位 iOS 仓库 + 审计):前置完成 —— iOS 仓库在 `/Users/minliny/Documents/Reader for iOS`,分支 `codex/ios-real-app-core-evidence`,96 文件 import 旧 Swift Core(大部分只模型,非服务)
- [x] Step 2(审计 Rust Core iOS binding 就绪度):前置完成 —— binding 就绪,XCFramework + Swift wrapper + shell smoke 33/33 PASS,无 release blocker
- [x] Step 3(退役方案):Task 1-7 覆盖
- [x] Step 4(执行):Task 1-7 是执行步骤,Task 8 是验证 + 提交
- [x] Step 5(提交):每个 Task 末尾有 commit step,Task 8 Step 3 提交 plan 到主仓库
- [x] 约束 Core/Host 边界:iOS 仅保留 URLSession + WebView + Keychain + AVSpeech(红线 4)
- [x] 约束 不破坏纯 UI:35 个 SwiftUI 页面不动,只改 ShellAssembly/ReaderApp/Provider
- [x] 约束 证据分层:每个 commit 标注 wrapper smoke / App-sim proof / 非 real device
- [x] 章程 §9 五问:Task 8 Step 1

### 2. Placeholder scan

- Task 4 Step 2 的 `// ... 其余 controlledOnline/OfflineReplay/mock 分支不动` 是省略,实际是用 Edit 工具在 `getBookDetail` 开头插入 rustCore 分支,其余分支保持原样 —— 编辑指令已明确,非 placeholder
- Task 7 的 `testBookSourceJSON` 用 `https://www.biquges123.com` —— 若该站失效,E2E 测试会失败,需换源。这是 E2E 测试的固有特性,非 placeholder;`SKIP_NETWORK_TESTS` 环境变量提供 offline 降级
- Task 7 Step 5 的模拟器启动命令 —— 若 `iPhone 17 Pro` 不存在,需用 `xcrun simctl list devices available` 找替代;这是环境依赖,非 placeholder

### 3. Type consistency

- `RustCoreSearchService/TOCService/ContentService` 构造签名 `init(runtime: ReaderCoreNativeRuntime, router: HostRequestRouter? = nil, requestTimeout: TimeInterval = 15)` —— Task 4 的 `RustCoreBookDetailService` 用相同签名,一致
- `RustCoreServiceSupport.serializeSource(_ source: BookSource) -> [String: Any]` —— Task 4 复用,一致
- `RustCoreServiceSupport.pollEvent(runtime:requestId:timeout:)` —— Task 4 复用,一致
- `RustCoreServiceSupport.mapCoreError(_ error:)` —— Task 4 复用,一致
- `SearchResultItem(title:detailURL:author:coverURL:intro:)` —— Task 4 的 `parseBookDetail` 用此初始化器,与 `RustCoreSearchService.parseBooks` 一致
- `ProviderBackedSearchService/TOCService/ContentService` 重命名后 —— Task 6 测试引用一致更新

### 4. 已知风险

1. **模拟器网络**:E2E 测试需真实网络访问,CI 环境可能不稳定 → `SKIP_NETWORK_TESTS` 环境变量降级
2. **书源规则失效**:`biquges123.com` 可能改版 → 失败时换源(用 `AppSupport/Sources/sample_book_source.json` 或 `xingxingxsw.search-only.json`)
3. **ReaderApp target 预存问题**:STATUS.md Round 6 记录的 `BrightnessPolicy` 跨模块可见性 —— 若 `xcodebuild build -scheme ReaderForIOSApp` 因此失败,需用独立 scheme `ReaderCoreNativeAdapterSmokeTests` 验证 adapter 层,App build 留 S7 修复
4. **MockReaderCoreService 仍被测试引用**:本方案不删 `MockReaderCoreService.swift`,只从生产路径退役 —— 测试仍可注入 mock 场景(如 `MockScenario.parserFailure`)
5. **ReaderCoreServices import 未全清**:`Default*Service.swift` 标记 deprecated 后,`import ReaderCoreServices` 仍在;S7 才真正删除文件 + 清 import
6. **useRealServices UserDefaults 残留**:Task 2 移除 App 代码引用,但已存设备的 UserDefaults 仍有该 key —— 无害(无人读),S7 清理
7. **ReaderCoreServiceProvider 多模式分支保留**:Task 3 只改默认值 + 加 rustCore 诊断,`canUseRealService`/`controlledOnline`/`offlineReplay`/`mock` 分支保留 —— 测试仍需这些模式。生产路径因默认 `.rustCore` 不会落到这些分支

### 5. 后续迭代(S7+)

- 真机 device proof(章程 §10.3 真实证据)
- 删除 `Default{Search,TOC,Content}Service.swift` + `MockReaderCoreService.swift` + `makeRealReadingFlowCoordinator`(strangler 完成)
- 清除所有 `import ReaderCoreServices`(保留 `ReaderCoreModels`/`ReaderCoreProtocols`)
- WebView host capability(`webview.evaluateJavaScript`)— 支持含 webView/webJs 的 Legado 书源
- TTS(系统 TTS + HttpTTS,章程 TTS 策略)
- 本地书(EPUB/TXT/PDF,Core 已支持 local_book.parse)
- RSS 订阅 + WebDAV 同步经 Rust Core
- 主题绘制
- corpus benchmark 四端同结果(章程 §9 主线不变量)
- 修复 `ReaderApp` target `BrightnessPolicy` 跨模块可见性(预存问题)
