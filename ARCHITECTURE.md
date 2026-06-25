# Reader-Core-Native Architecture

This document describes the current code architecture in branch HEAD. The full
product goal and development route are maintained in
`docs/FULL_DEVELOPMENT_ROADMAP.md`.

A capability is marked complete here only when it has a code path or
verification command in this repository, or a named branch fact verified by Git.
Roadmap-level parity claims still require the Legado capability ledger,
Reader-Core migration ledger, Native/C ABI evidence, and corpus benchmark proof
defined by the full roadmap.

## Current Baseline

The branch baseline is `origin/codex/core-product-integration` at `fb4c3a7`.
Verified branch facts:

```bash
git log --oneline --first-parent -n 20 origin/codex/core-product-integration
git branch -r --contains origin/codex/core-product-integration
git log --oneline HEAD..origin/codex/android-integration --
git ls-tree -r HEAD | rg 'bindings/android|build-android-jni'
git ls-tree -r origin/codex/android-jni-smoke | rg 'bindings/android|build-android-jni'
```

Observed state:

- `remote-reading-vertical`, `http-host-contract`, `cli-host-http-smoke`,
  `ios-swift-client-smoke`, `ios-xcframework-smoke`,
  `ios-swift-wrapper-smoke`, `quickjs-runtime`, `rule-engine-nonjs`, and
  `rule-engine-edgecases` are merged into the current Core product baseline.
- `origin/codex/android-integration` contains the current baseline plus
  `origin/codex/android-jni-smoke`; the Android JNI files are not present in
  branch HEAD.
- The repository tree has iOS and Harmony binding lanes, but Android is only
  `bindings/android/.gitkeep` in this baseline.

## Runtime Shape

```text
Platform app
  -> Swift / JNI / NAPI host wrapper
  -> include/reader_core.h C ABI
  -> crates/reader-ffi
  -> crates/reader-runtime worker
  -> crates/reader-contract command/event DTOs
  -> domain, rule, JS, content, storage modules
  -> host.request events for platform-owned capabilities
```

The ABI exposes a single opaque runtime handle and JSON command/event messages.
Evidence:

- `include/reader_core.h`
- `crates/reader-ffi/src/lib.rs`
- `crates/reader-ffi/src/runtime.rs`
- `crates/reader-runtime/src/runtime.rs`

Exported ABI v1 functions:

- `rc_abi_version`
- `rc_runtime_create`
- `rc_runtime_send`
- `rc_runtime_cancel`
- `rc_runtime_destroy`

There is no `rc_buffer_free` in ABI v1. Event callback buffers are borrowed for
the callback duration only; hosts must copy bytes they keep.

## Protocol And Capabilities

Protocol v1 is represented by `crates/reader-contract` and the JSON schemas in
`protocol/`.

Evidence:

- `crates/reader-contract/src/lib.rs`
- `crates/reader-contract/src/command.rs`
- `crates/reader-contract/src/event.rs`
- `protocol/reader-command.schema.json`
- `protocol/reader-event.schema.json`

Advertised v1 capabilities:

- `core.info`
- `runtime.ping`
- `runtime.hostSmoke`
- `host.complete`
- `host.error`
- `host.bus.v1`
- `http.execute`
- `runtime.config.v1`
- `remote.reading.v1`

`core.ping` remains accepted only as a bootstrap alias in runtime dispatch; it
is intentionally absent from `core.info.capabilities` and command-schema method
examples.

## Implemented Core-Owned Capabilities

| Capability | Current status | Evidence |
| --- | --- | --- |
| Command/event protocol v1 | Implemented | `crates/reader-contract/src/*.rs`, `protocol/*.schema.json`, `cargo run -p reader-cli -- --conformance` |
| Runtime worker, request IDs, duplicate-active rejection, cancellation | Implemented | `crates/reader-runtime/src/runtime.rs`, `cargo test -p reader-runtime` |
| Host bus request/complete/error routing | Implemented | `crates/reader-runtime/src/runtime.rs`, `protocol/fixtures/conformance/host/*.json`, `cargo run -p reader-cli -- --host-smoke` |
| C ABI lifecycle | Implemented | `include/reader_core.h`, `crates/reader-ffi/src/*.rs`, `./scripts/ffi-smoke.sh` |
| Non-JS rule primitives | Implemented for V1 primitive set: regex, JSONPath, CSS text/attr, XPath, fallback, chaining | `crates/reader-rule/src/lib.rs`, `crates/reader-rule/tests/*.rs`, `cargo test -p reader-rule` |
| QuickJS sandbox | Implemented as a sandbox with JSON conversion, console capture, timeout/cancel support, and host callback registry | `crates/reader-js/src/lib.rs`, `cargo test -p reader-js` |
| Remote-reading V1 vertical | Implemented for source import, search, detail, toc, chapter content, progress update | `crates/reader-runtime/src/remote.rs`, `tools/reader-cli/tests/fixture_vertical.rs`, `cargo run -p reader-cli -- --fixture-vertical tests/fixtures/remote_source/basic_source.json` |
| HTTP transport contract | Implemented as host capability request/completion, not as Core sockets | `crates/reader-contract/src/remote.rs`, `crates/reader-runtime/src/remote.rs`, `protocol/compatibility.md` |
| V1 cache/progress/source/book storage | Implemented in memory only | `crates/reader-storage/src/lib.rs` |
| iOS Core-side wrapper smoke | Implemented for ABI lifecycle, `core.info`, `runtime.ping` | `bindings/ios/Sources/ReaderCoreClient/ReaderCoreClient.swift`, `scripts/check-ios-swift-wrapper.sh` |
| Harmony Core-side NAPI build smoke | Implemented as build lane requiring OHOS SDK | `bindings/harmony/native/reader_napi.cpp`, `scripts/build-harmony-napi.sh` |

## Known Gaps

These are not product-complete in the current baseline, and each gap has a code
or branch reason:

| Gap | Owner | Evidence |
| --- | --- | --- |
| SQLite-backed persistent store, schema migration, durable cache/progress | Core | `crates/reader-storage/src/lib.rs` documents V1 in-memory only |
| Applying runtime config through the C ABI create path | Core | `crates/reader-ffi/src/runtime.rs` ignores `_config_json`/`_config_length`; pure Rust config exists in `crates/reader-contract/src/config.rs` |
| TXT/EPUB parsing | Core | `crates/reader-local-book/src/lib.rs` is a placeholder |
| RSS parsing/subscription state | Core | `crates/reader-rss/src/lib.rs` is a placeholder |
| WebDAV/sync/backup/conflict logic | Core | `crates/reader-sync/src/lib.rs` is a placeholder |
| Full Legado/Swift Core rule parity | Core | Current evidence covers primitive rule tests and V1 fixture vertical only; no sample-corpus parity runner is present |
| JS network through runtime host bus | Core plus host | JS sandbox supports host callbacks, but remote V1 default path reports `unsupported` for unregistered `java.get`/`java.post`; see `crates/reader-content/src/lib.rs` and `tools/reader-cli/tests/fixture_vertical.rs` |
| Real HTTP/TLS/socket behavior | Host/app | Core emits `http.execute`; platform hosts must execute sockets and return `host.complete` |
| WebView login, captcha, cookie extraction | Host/app | No WebView adapter exists in this Core repo; only host capability protocol exists |
| Keychain/Keystore/Secure Store | Host/app | No secure storage adapter exists in this Core repo |
| File picker and sandbox permission grants | Host/app | No platform file picker code exists in this Core repo |
| TTS playback, notifications, UI/navigation/theme | Host/app | Out of Core tree by design |
| Android JNI in current Core baseline | Integration pending | Current `HEAD` only has `bindings/android/.gitkeep`; files exist on `origin/codex/android-jni-smoke` |
| App-side iOS/Harmony/Android runtime integration | Host/app | This repo has Core-side smokes only; app repos own loading, lifecycle, adapters, and UI flows |

## Core-Owned Versus Host-Owned Boundary

Core owns deterministic product semantics:

- Protocol DTOs, method names, capability advertisement, structured errors.
- Rule execution primitives and content extraction.
- QuickJS sandbox behavior and structured unsupported errors.
- Remote-reading state machine and host operation continuation.
- Storage semantics once persistent storage is implemented.
- Source/book/toc/chapter/progress domain models.

Host/app owns platform capability and user experience:

- HTTP/TLS/socket execution and platform network policy.
- WebView login, captcha, cookie capture, and platform session stores.
- Secure credential storage.
- File picker, sandbox grants, and platform document access.
- TTS audio playback and platform media session.
- Background execution, notifications, app lifecycle, UI, navigation, theme.
- App packaging, signing, crash reporting, and store distribution.

The boundary is enforced in current code by the host bus: Core emits
`host.request` and never opens a socket itself. Hosts answer with
`host.complete` or `host.error`.

## Verification Commands

Core-local gates:

```bash
cargo fmt --check
cargo test --workspace
cargo run -p reader-cli -- --conformance
cargo run -p reader-cli -- --host-smoke
cargo run -p reader-cli -- --fixture-vertical tests/fixtures/remote_source/basic_source.json
./scripts/ffi-smoke.sh
./scripts/build-local.sh
```

Platform-smoke gates:

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
./scripts/check-ios-swift-wrapper.sh

OHOS_SDK_HOME=/path/to/ohos-sdk ./scripts/build-harmony-napi.sh
```

Branch-fact gates:

```bash
git merge-base --is-ancestor origin/codex/android-jni-smoke HEAD
git log --oneline HEAD..origin/codex/android-integration --
git ls-tree -r origin/codex/android-jni-smoke | rg 'bindings/android|build-android-jni'
```

In this checkout, the first command returns non-zero because Android JNI smoke
is not merged into the current Core product baseline.
