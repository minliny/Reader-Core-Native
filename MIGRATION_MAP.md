# Migration Map

This map records what has actually landed in the current Core product baseline
and what remains Core-owned versus host/app-owned work. Claims are backed by
paths or commands in this repository.

## Baseline

Current branch was created from `origin/codex/core-product-integration`
(`fb4c3a7` locally). The baseline includes Core protocol/runtime, FFI, rule/JS
foundation, remote-reading V1, host HTTP contract, CLI smoke, and iOS wrapper
smoke.

Evidence commands:

```bash
git log --oneline --first-parent -n 20 origin/codex/core-product-integration
git branch -r --contains origin/codex/core-product-integration
```

## Phase Status

| Area | Status | Evidence |
| --- | --- | --- |
| Core protocol and runtime | Done for v1 | `crates/reader-contract/src/lib.rs`, `crates/reader-runtime/src/runtime.rs`, `cargo run -p reader-cli -- --conformance` |
| C ABI foundation | Done for v1 lifecycle | `include/reader_core.h`, `crates/reader-ffi/src/lib.rs`, `./scripts/ffi-smoke.sh` |
| Non-JS rules and QuickJS sandbox | Partial product foundation | `crates/reader-rule/tests/*.rs`, `crates/reader-js/src/lib.rs`, `cargo test -p reader-rule -p reader-js` |
| Remote-reading V1 vertical | Done for fixture/inline/host-complete smoke | `crates/reader-runtime/src/remote.rs`, `tools/reader-cli/tests/fixture_vertical.rs` |
| Storage/cache/progress | In-memory V1 only | `crates/reader-storage/src/lib.rs` |
| Persistent SQLite and migrations | Not implemented | No SQLite backend in `crates/reader-storage`; crate documents deferred backend |
| Local book | Not implemented | `crates/reader-local-book/src/lib.rs` placeholder |
| RSS | Not implemented | `crates/reader-rss/src/lib.rs` placeholder |
| Sync/WebDAV/backup | Not implemented | `crates/reader-sync/src/lib.rs` placeholder |

## Platform Migration

### HarmonyOS

| Item | Status | Evidence |
| --- | --- | --- |
| Core-side NAPI build lane | Present | `bindings/harmony/native/reader_napi.cpp`, `bindings/harmony/native/CMakeLists.txt`, `scripts/build-harmony-napi.sh` |
| OHOS Rust staticlib build lane | Present | `scripts/build-ohos.sh` |
| App-side HAP integration and device runtime | Host/app pending | No app repo code in this Core tree; build script only produces Core-side native artifact |
| ArkTS wrapper, HTTP/WebView/TTS adapters | Host/app pending | No ArkTS adapter code in this Core tree |

Validation command when OHOS SDK is installed:

```bash
OHOS_SDK_HOME=/path/to/ohos-sdk ./scripts/build-harmony-napi.sh
```

### iOS

| Item | Status | Evidence |
| --- | --- | --- |
| XCFramework/header/modulemap smoke | Present | `scripts/build-ios-xcframework.sh`, `bindings/ios/module.modulemap`, `bindings/ios/README.md` |
| Swift wrapper lifecycle, `core.info`, `runtime.ping` | Present | `bindings/ios/Sources/ReaderCoreClient/ReaderCoreClient.swift`, `scripts/check-ios-swift-wrapper.sh` |
| URLSession host transport, WebView login, app UI integration | Host/app pending | `bindings/ios/README.md` explicitly scopes them to the iOS host repository |

Validation command when Xcode and iOS Rust targets are installed:

```bash
./scripts/check-ios-swift-wrapper.sh
```

### Android

| Item | Status | Evidence |
| --- | --- | --- |
| Android JNI in current Core product baseline | Not present | `git ls-tree -r HEAD | rg 'bindings/android|build-android-jni'` shows only `bindings/android/.gitkeep` |
| Android JNI smoke branch | Exists outside current baseline | `origin/codex/android-jni-smoke` contains `bindings/android/jni/reader_jni.cpp` and `scripts/build-android-jni.sh` |
| Android integration branch | Exists outside current baseline | `origin/codex/android-integration` contains current baseline plus `f205b2d feat: add android jni smoke build` |
| OkHttp/WebView/TTS/Room/app UI migration | Host/app pending | No Android host adapter implementation in branch HEAD |

Branch-fact commands:

```bash
git merge-base --is-ancestor origin/codex/android-jni-smoke HEAD
git log --oneline HEAD..origin/codex/android-integration --
git ls-tree -r origin/codex/android-jni-smoke | rg 'bindings/android|build-android-jni'
```

Expected current-baseline result: the merge-base command is non-zero because
Android JNI smoke has not landed in `origin/codex/core-product-integration`.

## Next Core-Owned Gaps

These are product gaps owned by this repo, not platform hosts:

- Route `config_json` from `rc_runtime_create` into `Runtime::new_with_config_json`
  and return structured ABI status for invalid config.
- Replace V1 `InMemoryStorage` with a persistent SQLite-backed backend while
  preserving the runtime-facing storage API.
- Add real TXT/EPUB, RSS, and sync/WebDAV implementations where placeholder
  crates exist today.
- Expand rule compatibility beyond primitive V1 tests with a sample-corpus
  runner that exercises source behavior through `reader-cli`.
- Bridge JS host callbacks to the runtime host bus instead of only reporting
  unregistered `java.get`/`java.post` as unsupported in the default pipeline.

## Host/App-Owned Gaps

These remain outside this Core repo:

- Real socket/TLS execution for `http.execute`.
- WebView login/captcha and cookie extraction.
- Secure storage, file picker, sandbox permissions, TTS playback.
- App lifecycle, UI, navigation, theme, background work, notifications.
- Platform packaging, signing, store distribution, and device-level telemetry.

Last updated: 2026-06-24.
