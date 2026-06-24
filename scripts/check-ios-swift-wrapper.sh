#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")/.."

wrapper_source="bindings/ios/Sources/ReaderCoreClient/ReaderCoreClient.swift"
host_lib="${READER_CORE_HOST_LIB:-${CARGO_TARGET_DIR:-target}/debug/libreader_core.a}"

typecheck_wrapper() {
  local headers="$1"

  test -f "$headers/reader_core.h"
  test -f "$headers/module.modulemap"

  xcrun --sdk iphonesimulator swiftc \
    -target arm64-apple-ios13.0-simulator \
    -I "$headers" \
    -typecheck "$wrapper_source"

  echo "typechecked $wrapper_source"
}

typecheck_wrapper_with_repo_headers() (
  local headers
  headers="$(mktemp -d -t reader-core-swift-wrapper-headers)"
  trap 'rm -rf "$headers"' EXIT

  cp include/reader_core.h "$headers/reader_core.h"
  cp bindings/ios/module.modulemap "$headers/module.modulemap"
  typecheck_wrapper "$headers"
)

run_macos_swift_smoke() (
  local library="$1"
  if [[ ! -f "$library" ]]; then
    echo "missing host static library: $library" >&2
    echo "build it with: cargo build -p reader-ffi" >&2
    exit 1
  fi

  local tmp_dir host_headers smoke_source smoke_bin
  tmp_dir="$(mktemp -d -t reader-core-swift-client-smoke)"
  trap 'rm -rf "$tmp_dir"' EXIT

  host_headers="$tmp_dir/headers"
  smoke_source="$tmp_dir/client-smoke.swift"
  smoke_bin="$tmp_dir/client-smoke-bin"

  mkdir -p "$host_headers"
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

final class SmokeURLProtocol: URLProtocol {
    static var lastRequest: URLRequest?

    override class func canInit(with request: URLRequest) -> Bool {
        request.url?.host == "transport.example.test"
    }

    override class func canonicalRequest(for request: URLRequest) -> URLRequest {
        request
    }

    override func startLoading() {
        Self.lastRequest = request
        let response = HTTPURLResponse(
            url: request.url!,
            statusCode: 202,
            httpVersion: nil,
            headerFields: [
                "Content-Type": "application/json",
                "X-Smoke": "url-protocol",
            ]
        )!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data("{\"ok\":true}".utf8))
        client?.urlProtocolDidFinishLoading(self)
    }

    override func stopLoading() {}
}

final class HangingURLProtocol: URLProtocol {
    static var didStop = false

    override class func canInit(with request: URLRequest) -> Bool {
        request.url?.host == "timeout.example.test"
    }

    override class func canonicalRequest(for request: URLRequest) -> URLRequest {
        request
    }

    override func startLoading() {
        // Intentionally never calls the client; URLSessionHostTransport must
        // enforce its own timeout and cancel the task.
    }

    override func stopLoading() {
        Self.didStop = true
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

        // manual host.complete path ------------------------------------------------
        // runtime.hostSmoke is a local driver that emits host.request and resumes
        // only after the host sends host.complete for the operation.
        try client.send(
            method: "runtime.hostSmoke",
            requestId: 150,
            params: [
                "capability": "host.smoke.echo",
                "params": ["hello": "world"],
            ]
        )
        var hostRequestEvent: ReaderCoreEvent?
        let hostPollDeadline = Date().addingTimeInterval(5)
        while Date() < hostPollDeadline {
            if let result = client.pollEvent(requestId: 150) {
                let event = try result.get()
                if event.isHostRequest {
                    hostRequestEvent = event
                    break
                }
                throw SmokeFailure("runtime.hostSmoke produced unexpected event type \(event.type)")
            }
        }
        guard let hostRequestEvent else {
            throw SmokeFailure("runtime.hostSmoke did not emit host.request")
        }
        guard hostRequestEvent.capability == "host.smoke.echo" else {
            throw SmokeFailure("runtime.hostSmoke capability mismatch")
        }
        guard let operationId = hostRequestEvent.operationId, operationId > 0 else {
            throw SmokeFailure("runtime.hostSmoke operationId missing")
        }
        _ = try client.sendHostComplete(
            operationId: operationId,
            result: ["echoed": true],
            requestId: 151
        )
        var manualResult: ReaderCoreEvent?
        let manualDeadline = Date().addingTimeInterval(5)
        while Date() < manualDeadline {
            if let result = client.pollEvent(requestId: 150) {
                manualResult = try result.get()
                break
            }
        }
        guard let manualResult, manualResult.type == "result" else {
            throw SmokeFailure("manual host.complete did not resume original request")
        }
        guard (manualResult.data?["echoed"] as? Bool) == true else {
            throw SmokeFailure("manual host.complete result payload mismatch")
        }

        // Internal host-complete command IDs must not collide with host-provided
        // request IDs. This request intentionally uses 1001, the old low
        // internal counter's first generated ID.
        try client.send(
            method: "runtime.hostSmoke",
            requestId: 1001,
            params: [
                "capability": "host.smoke.echo",
                "params": ["collision": true],
            ]
        )
        var collisionHostRequest: ReaderCoreEvent?
        let collisionHostDeadline = Date().addingTimeInterval(5)
        while Date() < collisionHostDeadline {
            if let result = client.pollEvent(requestId: 1001) {
                let event = try result.get()
                if event.isHostRequest {
                    collisionHostRequest = event
                    break
                }
                throw SmokeFailure("collision smoke produced unexpected event type \(event.type)")
            }
        }
        guard let collisionOperationId = collisionHostRequest?.operationId, collisionOperationId > 0 else {
            throw SmokeFailure("collision smoke operationId missing")
        }
        _ = try client.sendHostComplete(
            operationId: collisionOperationId,
            result: ["collision": "avoided"]
        )
        var collisionResult: ReaderCoreEvent?
        let collisionDeadline = Date().addingTimeInterval(5)
        while Date() < collisionDeadline {
            if let result = client.pollEvent(requestId: 1001) {
                collisionResult = try result.get()
                break
            }
        }
        guard let collisionResult, collisionResult.type == "result" else {
            throw SmokeFailure("internal host.complete command id collided with requestId 1001")
        }
        guard collisionResult.data?["collision"] as? String == "avoided" else {
            throw SmokeFailure("collision smoke result payload mismatch")
        }

        // cancel path --------------------------------------------------------------
        // Leave a host request pending, cancel the original request, and verify Core
        // emits the typed CANCELLED event through the same buffered event path.
        try client.send(
            method: "runtime.hostSmoke",
            requestId: 175,
            params: [
                "capability": "host.smoke.echo",
                "params": ["cancel": true],
            ]
        )
        var cancelHostRequest: ReaderCoreEvent?
        let cancelHostDeadline = Date().addingTimeInterval(5)
        while Date() < cancelHostDeadline {
            if let result = client.pollEvent(requestId: 175) {
                let event = try result.get()
                if event.isHostRequest {
                    cancelHostRequest = event
                    break
                }
                throw SmokeFailure("cancel smoke produced unexpected event type \(event.type)")
            }
        }
        guard cancelHostRequest?.operationId != nil else {
            throw SmokeFailure("cancel smoke did not produce host.request")
        }
        try client.cancel(requestId: 175)
        var cancelledEvent: ReaderCoreEvent?
        let cancelDeadline = Date().addingTimeInterval(5)
        while Date() < cancelDeadline {
            if let result = client.pollEvent(requestId: 175) {
                cancelledEvent = try result.get()
                break
            }
        }
        guard let cancelledEvent, cancelledEvent.type == "error" else {
            throw SmokeFailure("cancel did not produce an error event")
        }
        guard cancelledEvent.coreError?.code == "CANCELLED" else {
            throw SmokeFailure("cancel did not surface CANCELLED")
        }

        // default URLSession transport --------------------------------------------
        // Use URLProtocol instead of the network so the smoke proves the built-in
        // URLSessionHostTransport request/response mapping deterministically.
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [SmokeURLProtocol.self]
        let urlSessionTransport = URLSessionHostTransport(
            session: URLSession(configuration: configuration)
        )
        let urlSessionResponse = try urlSessionTransport.perform(ReaderCoreHostRequest(
            operationId: 176,
            capability: "http.execute",
            rawParams: [
                "url": "https://transport.example.test/books",
                "method": "POST",
                "headers": ["Accept": "application/json"],
                "body": "{\"q\":\"dune\"}",
            ]
        ))
        guard SmokeURLProtocol.lastRequest?.httpMethod == "POST" else {
            throw SmokeFailure("URLSessionHostTransport did not apply method")
        }
        guard SmokeURLProtocol.lastRequest?.value(forHTTPHeaderField: "Accept") == "application/json" else {
            throw SmokeFailure("URLSessionHostTransport did not apply headers")
        }
        guard urlSessionResponse.status == 202 else {
            throw SmokeFailure("URLSessionHostTransport status mismatch")
        }
        guard urlSessionResponse.headers["X-Smoke"] == "url-protocol" else {
            throw SmokeFailure("URLSessionHostTransport headers mismatch")
        }
        guard urlSessionResponse.body == "{\"ok\":true}" else {
            throw SmokeFailure("URLSessionHostTransport body mismatch")
        }

        let timeoutConfiguration = URLSessionConfiguration.ephemeral
        timeoutConfiguration.protocolClasses = [HangingURLProtocol.self]
        let timeoutTransport = URLSessionHostTransport(
            session: URLSession(configuration: timeoutConfiguration),
            timeout: 0.05
        )
        var timedOut = false
        do {
            _ = try timeoutTransport.perform(ReaderCoreHostRequest(
                operationId: 177,
                capability: "http.execute",
                rawParams: ["url": "https://timeout.example.test/hang"]
            ))
        } catch ReaderCoreClientError.hostTransportFailed(let message) {
            timedOut = message.contains("timed out")
        }
        guard timedOut else {
            throw SmokeFailure("URLSessionHostTransport did not time out")
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

        // FFI-level send error exposure: malformed command JSON surfaces -----
        // sendFailed carrying the coarse ABI status plus rc_last_error.
        let rawRuntime = try ReaderCoreRuntime(onEvent: { _ in })
        defer { rawRuntime.destroy() }
        do {
            try rawRuntime.send(jsonString: "{not valid json")
            throw SmokeFailure("malformed JSON send did not fail")
        } catch ReaderCoreClientError.sendFailed(let status, let lastError) {
            guard status == 3 else {
                throw SmokeFailure("malformed JSON send returned status \(status), expected 3")
            }
            guard let lastError else {
                throw SmokeFailure("malformed JSON send did not expose rc_last_error")
            }
            guard lastError.code == 5 else {
                throw SmokeFailure("malformed JSON rc_last_error code \(lastError.code), expected 5")
            }
            guard lastError.message.contains("invalid command JSON") else {
                throw SmokeFailure("malformed JSON rc_last_error message was \(lastError.message)")
            }
        }

        // FFI-level create error exposure: invalid config JSON surfaces ------
        // createFailed carrying the coarse ABI status plus rc_last_error.
        do {
            _ = try ReaderCoreRuntime(configJSON: Data("{".utf8), onEvent: { _ in })
            throw SmokeFailure("invalid config JSON create did not fail")
        } catch ReaderCoreClientError.createFailed(let status, let lastError) {
            guard status == 4 else {
                throw SmokeFailure("invalid config create returned status \(status), expected 4")
            }
            guard let lastError else {
                throw SmokeFailure("invalid config create did not expose rc_last_error")
            }
            guard lastError.code == 5 else {
                throw SmokeFailure("invalid config rc_last_error code \(lastError.code), expected 5")
            }
            guard !lastError.message.isEmpty else {
                throw SmokeFailure("invalid config rc_last_error message was empty")
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
    "$wrapper_source" \
    "$smoke_source" \
    "$library" \
    -o "$smoke_bin"

  "$smoke_bin"
)

if [[ "${READER_CORE_IOS_SWIFT_ONLY:-0}" == "1" ]]; then
  typecheck_wrapper_with_repo_headers
  run_macos_swift_smoke "$host_lib"
  echo "swift-only wrapper checks passed using existing $host_lib"
  exit 0
fi

if ! ./scripts/build-ios-xcframework.sh; then
  echo "full iOS Swift wrapper gate stopped while building the XCFramework" >&2
  echo "wrapper-only validation can be run explicitly with:" >&2
  echo "  READER_CORE_IOS_SWIFT_ONLY=1 bash ./scripts/check-ios-swift-wrapper.sh" >&2
  exit 1
fi

headers="target/ios/ReaderCore.xcframework/ios-arm64-simulator/Headers"
typecheck_wrapper "$headers"

if ! cargo build -p reader-ffi; then
  echo "full iOS Swift wrapper gate stopped while building the host static library" >&2
  echo "wrapper-only validation can be run explicitly with:" >&2
  echo "  READER_CORE_IOS_SWIFT_ONLY=1 bash ./scripts/check-ios-swift-wrapper.sh" >&2
  exit 1
fi

run_macos_swift_smoke "$host_lib"
