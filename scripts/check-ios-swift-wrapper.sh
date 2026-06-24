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

/// Stub host transport: answers every `http.execute` request with a canned
/// search-response body, proving the host.request/host.complete loop without
/// touching the network.
final class StubHostTransport: ReaderCoreHostTransport {
    var lastRequest: ReaderCoreHostRequest?

    func perform(_ request: ReaderCoreHostRequest) throws -> ReaderCoreHostResponse {
        lastRequest = request
        guard request.capability == "http.execute" else {
            throw SmokeFailure("unexpected host capability \(request.capability)")
        }
        guard request.url?.contains("books.example.test") == true else {
            throw SmokeFailure("unexpected host url \(request.url ?? "nil")")
        }
        guard request.method == "GET" else {
            throw SmokeFailure("unexpected host method \(request.method ?? "nil")")
        }
        guard request.headers["Accept"] == "application/json" else {
            throw SmokeFailure("missing Accept header")
        }
        return ReaderCoreHostResponse(
            status: 200,
            headers: ["Content-Type": "application/json"],
            body: "{\"books\":[{\"bookId\":\"1\",\"title\":\"Dune\",\"author\":\"Herbert\"}]}"
        )
    }
}

@main
struct ReaderCoreClientSmoke {
    static func main() throws {
        let stub = StubHostTransport()
        let client = try ReaderCoreClient(hostTransport: stub)
        defer { client.destroy() }

        guard ReaderCoreRuntime.abiVersion == 1 else {
            throw SmokeFailure("unexpected ABI version \(ReaderCoreRuntime.abiVersion)")
        }

        // core.info ----------------------------------------------------------------

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

        // runtime.ping -------------------------------------------------------------

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

        // polling path: drain core.info via pollEvent ------------------------------
        // Send without blocking, then poll the buffer until the result arrives.
        // This proves the non-blocking event-drainage path used by iOS run loops.
        try client.send(method: "core.info", requestId: 102)
        var polled: ReaderCoreEvent?
        let pollDeadline = Date().addingTimeInterval(5)
        while Date() < pollDeadline {
            if let result = client.pollEvent(requestId: 102) {
                polled = try result.get()
                break
            }
        }
        guard let polled, polled.type == "result" else {
            throw SmokeFailure("pollEvent did not observe a result event")
        }
        // A second poll must return nil: the event was already drained, and poll
        // must never block or replay consumed events.
        if client.pollEvent(requestId: 102) != nil {
            throw SmokeFailure("pollEvent returned an event that was already consumed")
        }

        // http.execute host flow ---------------------------------------------------
        // book.search with searchRequest (no searchResponse) forces Core to emit a
        // host.request { capability: "http.execute" }; the client drives the stub
        // transport and sends host.complete, then Core resumes and returns books.
        let searchSource: [String: Any] = [
            "sourceId": "vtest-src",
            "name": "Vertical Test Source",
            "baseUrl": "https://books.example.test",
            "rules": [
                "search": [["kind": "jsonPath", "path": "$.books[*]"]],
            ],
        ]
        let search = try client.request(
            method: "book.search",
            requestId: 200,
            params: [
                "sourceId": "vtest-src",
                "searchRequest": [
                    "url": "https://books.example.test/search?q=dune",
                    "headers": ["Accept": "application/json"],
                ],
                "source": searchSource,
            ],
            timeout: 5
        )
        guard search.type == "result", let searchData = search.data else {
            throw SmokeFailure("book.search did not return a result event")
        }
        guard let books = searchData["books"] as? [Any], books.count == 1 else {
            throw SmokeFailure("book.search did not return one book")
        }
        guard let firstBook = books[0] as? [String: Any], firstBook["title"] as? String == "Dune" else {
            throw SmokeFailure("book.search book title mismatch")
        }
        guard let captured = stub.lastRequest, captured.operationId > 0 else {
            throw SmokeFailure("host transport was not invoked")
        }

        // host.error path: a transport failure routes through host.error and the
        // originating command surfaces Core's error for the original requestId.
        let failingTransport = FailingHostTransport()
        let errorClient = try ReaderCoreClient(hostTransport: failingTransport)
        defer { errorClient.destroy() }
        var didThrow = false
        do {
            _ = try errorClient.request(
                method: "book.search",
                requestId: 201,
                params: [
                    "sourceId": "vtest-src",
                    "searchRequest": ["url": "https://books.example.test/search"],
                    "source": searchSource,
                ],
                timeout: 5
            )
        } catch ReaderCoreClientError.coreError(let coreError) {
            didThrow = true
            // Core resumes the original request after host.error; the surfaced
            // code/message are the host-error propagation, which must be non-empty.
            guard !coreError.message.isEmpty else {
                throw SmokeFailure("core error message was empty")
            }
        }
        guard didThrow else {
            throw SmokeFailure("transport failure did not surface as a core error")
        }

        // typed error exposure: unknown method yields UNKNOWN_METHOD ----------
        var unknownError: ReaderCoreCoreError?
        do {
            _ = try client.request(method: "no.such.method", requestId: 202, params: [:], timeout: 5)
        } catch ReaderCoreClientError.coreError(let coreError) {
            unknownError = coreError
        }
        guard let unknownError, unknownError.code == "UNKNOWN_METHOD" else {
            throw SmokeFailure("unknown method did not surface UNKNOWN_METHOD")
        }

        // FFI-level error exposure: malformed command JSON surfaces a --------
        // sendFailed carrying the coarse ABI status (3 = malformed message).
        let rawRuntime = try ReaderCoreRuntime(onEvent: { _ in })
        defer { rawRuntime.destroy() }
        do {
            try rawRuntime.send(jsonString: "{not valid json")
            throw SmokeFailure("malformed JSON send did not fail")
        } catch ReaderCoreClientError.sendFailed(let status) {
            guard status == 3 else {
                throw SmokeFailure("malformed JSON send returned status \(status), expected 3")
            }
        }

        print("swift client smoke passed")
    }
}

final class FailingHostTransport: ReaderCoreHostTransport {
    func perform(_ request: ReaderCoreHostRequest) throws -> ReaderCoreHostResponse {
        throw SmokeFailure("failing transport always fails")
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
