#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

./scripts/build-ios-xcframework.sh

headers="target/ios/ReaderCore.xcframework/ios-arm64-simulator/Headers"
test -f "$headers/reader_core.h"
test -f "$headers/module.modulemap"

xcrun --sdk iphonesimulator swiftc \
  -target arm64-apple-ios13.0-simulator \
  -I "$headers" \
  -typecheck bindings/ios/Sources/ReaderCoreClient/ReaderCoreClient.swift

echo "typechecked bindings/ios/Sources/ReaderCoreClient/ReaderCoreClient.swift"

cargo build -p reader-ffi

host_headers="$(mktemp -d -t reader-core-swift-host-headers)"
smoke_source="$(mktemp -t reader-core-swift-client-smoke).swift"
smoke_bin="$(mktemp -t reader-core-swift-client-smoke-bin)"
cleanup() {
  rm -rf "$host_headers"
  rm -f "$smoke_source" "$smoke_bin"
}
trap cleanup EXIT

cp include/reader_core.h "$host_headers/reader_core.h"
cp bindings/ios/module.modulemap "$host_headers/module.modulemap"

cat > "$smoke_source" <<'EOF'
import Foundation

struct SmokeFailure: Error, CustomStringConvertible {
    let description: String

    init(_ description: String) {
        self.description = description
    }
}

@main
struct ReaderCoreClientSmoke {
    static func main() throws {
        let client = try ReaderCoreClient()
        defer { client.destroy() }

        guard ReaderCoreRuntime.abiVersion == 1 else {
            throw SmokeFailure("unexpected ABI version \(ReaderCoreRuntime.abiVersion)")
        }

        let info = try client.coreInfo(requestId: 100, timeout: 5)
        guard info.type == "result", let infoData = info.data else {
            throw SmokeFailure("core.info did not return a result event")
        }
        guard (infoData["abiVersion"] as? NSNumber)?.uint32Value == ReaderCoreRuntime.abiVersion else {
            throw SmokeFailure("core.info ABI version mismatch")
        }
        let capabilities = infoData["capabilities"] as? [Any] ?? []
        guard capabilities.contains(where: { ($0 as? String) == "runtime.ping" }) else {
            throw SmokeFailure("core.info capabilities missing runtime.ping")
        }

        let ping = try client.ping(requestId: 101, timeout: 5)
        guard ping.type == "result", let pingData = ping.data else {
            throw SmokeFailure("runtime.ping did not return a result event")
        }
        guard (pingData["pong"] as? Bool) == true else {
            throw SmokeFailure("runtime.ping missing pong=true")
        }
        guard (pingData["method"] as? String) == "runtime.ping" else {
            throw SmokeFailure("runtime.ping method echo mismatch")
        }

        print("swift client smoke passed")
    }
}
EOF

swiftc \
  -I "$host_headers" \
  bindings/ios/Sources/ReaderCoreClient/ReaderCoreClient.swift \
  "$smoke_source" \
  target/debug/libreader_core.a \
  -o "$smoke_bin"

"$smoke_bin"
