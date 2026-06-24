# Reader-Core-Native

Reader three-platform Core implemented in Rust. This repository is the Core
product baseline: domain protocol, rule/content/runtime crates, C ABI, CLI
driver, and smoke-level iOS/Harmony binding artifacts.

The source of truth for current status is the checked-in code plus the
verification commands below. Older platform roadmap documents are not used as
evidence for completed capability.

## Quick Start

```bash
# local format + workspace tests
./scripts/check-local.sh

# build workspace, release C ABI staticlib, CLI info, and C/C++ ABI smoke
./scripts/build-local.sh

# protocol fixture conformance report
cargo run -p reader-cli -- --conformance

# remote-reading V1 fixture vertical
cargo run -p reader-cli -- --fixture-vertical tests/fixtures/remote_source/basic_source.json

# C and C++ host ABI smoke
./scripts/ffi-smoke.sh
```

Environment-gated platform smoke:

```bash
# OHOS SDK required: OHOS_SDK_HOME must point at an OpenHarmony SDK
./scripts/build-harmony-napi.sh

# Xcode and iOS Rust targets required
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
./scripts/check-ios-swift-wrapper.sh
```

## Current Product State

`origin/codex/core-product-integration` is at `fb4c3a7` for this branch HEAD. It has
merged the Core foundation, QuickJS/rule batches, remote-reading V1 vertical,
HTTP host contract, protocol/docs status, CLI host HTTP smoke, and iOS wrapper
smoke. Evidence:

- `crates/reader-contract/src/lib.rs` advertises ABI/protocol v1 capabilities:
  `core.info`, `runtime.ping`, `runtime.hostSmoke`, `host.complete`,
  `host.error`, `host.bus.v1`, `http.execute`, `runtime.config.v1`, and
  `remote.reading.v1`.
- `crates/reader-runtime/src/remote.rs` implements `source.import`,
  `book.search`, `book.detail`, `book.toc`, `chapter.content`, and
  `reading.progress.update` over inline/fixture responses or host-provided
  `http.execute` completions.
- `tools/reader-cli/tests/fixture_vertical.rs` drives the import -> search ->
  host HTTP search -> detail -> toc -> chapter -> progress path and verifies the
  JS-network-unsupported error path.
- `include/reader_core.h` and `crates/reader-ffi/src/lib.rs` expose the ABI v1
  runtime lifecycle: create, send, cancel, destroy, and ABI version.
- `bindings/ios/Sources/ReaderCoreClient/ReaderCoreClient.swift` wraps ABI
  lifecycle plus `core.info` and `runtime.ping`; `scripts/check-ios-swift-wrapper.sh`
  is the compile/link/runtime smoke gate.
- `bindings/harmony/native/reader_napi.cpp` and `scripts/build-harmony-napi.sh`
  build the Core-side Harmony NAPI `.so` when OHOS SDK tooling is present.

Current limits are also code-backed:

- Storage is V1 in-memory only. `crates/reader-storage/src/lib.rs` says the real
  SQLite-backed store is deferred, and no SQLite backend is implemented.
- Local book, RSS, and sync crates are placeholders:
  `crates/reader-local-book/src/lib.rs`, `crates/reader-rss/src/lib.rs`, and
  `crates/reader-sync/src/lib.rs`.
- Core never opens sockets. Remote reading emits `host.request` with
  `capability: "http.execute"` and resumes only after the host sends
  `host.complete`; see `crates/reader-runtime/src/remote.rs`.
- Runtime config schema exists, and the pure Rust runtime can parse it, but the
  current C ABI create path ignores `_config_json` and creates `Runtime::new`.
  See `crates/reader-contract/src/config.rs` and `crates/reader-ffi/src/runtime.rs`.
- Android JNI smoke exists as a branch fact on `origin/codex/android-jni-smoke`
  and is merged into `origin/codex/android-integration`, but it is not in this
  `origin/codex/core-product-integration` baseline. In branch HEAD, `HEAD`
  only contains `bindings/android/.gitkeep`.

## Documentation

- [ARCHITECTURE.md](./ARCHITECTURE.md) - current architecture, product status,
  and Core/host ownership boundary.
- [FEATURE_MATRIX.md](./FEATURE_MATRIX.md) - current capability matrix with
  evidence paths.
- [MIGRATION_MAP.md](./MIGRATION_MAP.md) - branch-aware platform migration map.
- [protocol/compatibility.md](./protocol/compatibility.md) - ABI/protocol v1
  compatibility contract.
- [docs/ROLLING_INTEGRATION.md](./docs/ROLLING_INTEGRATION.md) - branch
  integration helper notes.

## Repository Roles

```text
Reader-Core-Native       Rust Core, C ABI, protocol, CLI, Core-side smokes
Reader-for-iOS           UI plus Apple host adapters and app integration
Reader-for-Android       UI plus Android host adapters and app integration
Reader-for-HarmonyOS     UI plus ArkTS/NAPI host adapters and app integration
Reader-Core (Swift)      Frozen reference, not the target runtime Core
```
