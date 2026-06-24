# Reader-Core-Native Feature Matrix

Status values:

- Done: implemented in current `origin/codex/core-product-integration` baseline
  and backed by a path or command.
- Partial: useful foundation exists, but the product capability is not complete.
- Gap: not implemented in this baseline.
- Host/app: intentionally owned outside Core.

## Core Protocol And Runtime

| Capability | Owner | Status | Evidence |
| --- | --- | --- | --- |
| ABI v1 lifecycle | Core | Done | `include/reader_core.h`, `crates/reader-ffi/src/lib.rs`, `./scripts/ffi-smoke.sh` |
| JSON protocol v1 command/event DTOs | Core | Done | `crates/reader-contract/src/command.rs`, `crates/reader-contract/src/event.rs`, `protocol/*.schema.json` |
| Capability advertisement via `core.info` | Core | Done | `crates/reader-contract/src/core_info.rs`, `crates/reader-contract/src/lib.rs` |
| Runtime worker and request dispatch | Core | Done | `crates/reader-runtime/src/runtime.rs` |
| Cancel pending request | Core | Done | `crates/reader-runtime/src/runtime.rs`, `cargo run -p reader-cli -- --conformance` |
| Host bus `host.request` / `host.complete` / `host.error` | Core protocol, host execution | Done for routing | `crates/reader-runtime/src/runtime.rs`, `protocol/fixtures/conformance/host/*.json` |
| Runtime config schema | Core | Partial | `protocol/reader-runtime-config.schema.json`, `crates/reader-contract/src/config.rs`; C ABI create path currently ignores config bytes in `crates/reader-ffi/src/runtime.rs` |

## Remote Reading V1

| Capability | Owner | Status | Evidence |
| --- | --- | --- | --- |
| `remote.reading.v1` capability | Core | Done | `crates/reader-contract/src/lib.rs` |
| `source.import` | Core | Done | `crates/reader-runtime/src/remote.rs`, `tools/reader-cli/tests/fixture_vertical.rs` |
| `book.search` with prefetched response | Core | Done | `crates/reader-runtime/src/remote.rs`, `tests/fixtures/remote_source/basic_source.json` |
| `book.search` via `http.execute` host completion | Core protocol, host transport | Done for contract | `crates/reader-runtime/src/remote.rs`, `tools/reader-cli/tests/fixture_vertical.rs` |
| `book.detail` | Core | Done for V1 merge path | `crates/reader-runtime/src/remote.rs` |
| `book.toc` | Core | Done for V1 extraction path | `crates/reader-runtime/src/remote.rs` |
| `chapter.content` rule path | Core | Done for V1 extraction path | `crates/reader-runtime/src/remote.rs`, `crates/reader-content/src/lib.rs` |
| `chapter.content` JS network path | Core plus host | Partial | Default runtime reports structured unsupported for unregistered `java.get`/`java.post`; see `crates/reader-content/src/lib.rs` and fixture vertical test |
| `reading.progress.update` | Core | Done for in-memory V1 | `crates/reader-runtime/src/remote.rs`, `crates/reader-storage/src/lib.rs` |
| Chapter/content cache | Core | Partial | V1 writes in-memory cache only in `crates/reader-storage/src/lib.rs` |

## Rule, JS, Content

| Capability | Owner | Status | Evidence |
| --- | --- | --- | --- |
| Regex extract/replace | Core | Done for primitive engine | `crates/reader-rule/src/lib.rs`, `crates/reader-rule/tests/*.rs` |
| JSONPath lookup | Core | Done for implemented subset | `crates/reader-rule/src/lib.rs`, `crates/reader-rule/tests/rule_edgecases.rs` |
| CSS selector text/attr | Core | Done for implemented subset | `crates/reader-rule/src/lib.rs`, `crates/reader-rule/tests/*.rs` |
| XPath extraction and namespaces | Core | Done for implemented subset | `crates/reader-rule/src/lib.rs`, `crates/reader-rule/tests/rule_edgecases.rs` |
| Rule chaining/fallback | Core | Done for primitive engine | `crates/reader-rule/src/lib.rs`, `crates/reader-rule/tests/*.rs` |
| Full Legado/Swift rule compatibility | Core | Gap | No sample-corpus parity runner or full compatibility report in this baseline |
| QuickJS evaluation | Core | Done for sandbox foundation | `crates/reader-js/src/lib.rs`, `cargo test -p reader-js` |
| JS timeout/cancel/console/host callback registry | Core | Done for sandbox foundation | `crates/reader-js/src/lib.rs` |
| JS host callback integration with runtime host bus | Core plus host | Gap | Sandbox callbacks exist, but remote runtime default path does not bridge JS network to `http.execute` |
| HTML/XML/JSON content extraction for remote V1 | Core | Partial | `crates/reader-content/src/lib.rs`; V1 fixture only, not full source corpus |

## Storage And Data

| Capability | Owner | Status | Evidence |
| --- | --- | --- | --- |
| Source registry | Core | Done for in-memory V1 | `crates/reader-storage/src/lib.rs` |
| Book cache | Core | Done for in-memory V1 | `crates/reader-storage/src/lib.rs` |
| Chapter cache | Core | Done for in-memory V1 | `crates/reader-runtime/src/remote.rs`, `crates/reader-storage/src/lib.rs` |
| Reading progress | Core | Done for in-memory V1 | `crates/reader-storage/src/lib.rs` |
| SQLite schema and migrations | Core | Gap | `crates/reader-storage/src/lib.rs` documents deferred SQLite backend |
| Cookie/session persistence | Core protocol plus host capture | Gap | No cookie/session persistence implementation in this baseline |
| Download queue/recent history/offline cache | Core | Gap | No implementation in current crates |

## Platform And Host Capabilities

| Capability | Owner | Status | Evidence |
| --- | --- | --- | --- |
| Real HTTP/TLS/socket execution | Host/app | Host/app | Core only emits `http.execute`; see `crates/reader-runtime/src/remote.rs` |
| WebView login/captcha/cookie capture | Host/app | Host/app | No WebView adapter in Core tree |
| Secure credentials | Host/app | Host/app | No Keychain/Keystore/Secure Store code in Core tree |
| File picker and sandbox grants | Host/app | Host/app | No platform file picker code in Core tree |
| TTS audio playback | Host/app | Host/app | No platform media code in Core tree |
| UI/navigation/theme/font/background/notification | Host/app | Host/app | Not part of Core tree |

## Local Book, RSS, Sync

| Capability | Owner | Status | Evidence |
| --- | --- | --- | --- |
| TXT parsing | Core | Gap | `crates/reader-local-book/src/lib.rs` placeholder |
| EPUB parsing | Core | Gap | `crates/reader-local-book/src/lib.rs` placeholder |
| RSS parsing/subscription state | Core | Gap | `crates/reader-rss/src/lib.rs` placeholder |
| WebDAV/sync/backup/conflict | Core | Gap | `crates/reader-sync/src/lib.rs` placeholder |

## Platform Binding Lanes

| Capability | Owner | Status | Evidence |
| --- | --- | --- | --- |
| iOS XCFramework/header/modulemap smoke | Core | Done | `scripts/build-ios-xcframework.sh`, `bindings/ios/README.md` |
| iOS Swift wrapper smoke | Core | Done for lifecycle/info/ping | `bindings/ios/Sources/ReaderCoreClient/ReaderCoreClient.swift`, `scripts/check-ios-swift-wrapper.sh` |
| Harmony NAPI native build smoke | Core | Done for build lane | `bindings/harmony/native/reader_napi.cpp`, `scripts/build-harmony-napi.sh` |
| Android JNI in this baseline | Core integration | Gap | `HEAD` only contains `bindings/android/.gitkeep` |
| Android JNI smoke branch | Core integration branch fact | Partial outside baseline | `origin/codex/android-jni-smoke` contains `bindings/android/jni/reader_jni.cpp` and `scripts/build-android-jni.sh` |

Last updated: 2026-06-24.
