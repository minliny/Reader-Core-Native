#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")/.."

wrapper_source="bindings/ios/Sources/ReaderCoreClient/ReaderCoreClient.swift"
host_lib="${READER_CORE_HOST_LIB:-${CARGO_TARGET_DIR:-target}/debug/libreader_core.a}"
swift_only=0

if [[ "${1:-}" == "--swift-only" ]]; then
  swift_only=1
  shift
fi

if (( $# > 0 )); then
  echo "usage: bash ./scripts/check-ios-swift-wrapper.sh [--swift-only]" >&2
  exit 2
fi

if [[ "${READER_CORE_IOS_SWIFT_ONLY:-0}" == "1" ]]; then
  swift_only=1
fi

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

final class FailingHostTransport: ReaderCoreHostTransport {
    func perform(_ request: ReaderCoreHostRequest) throws -> ReaderCoreHostResponse {
        throw SmokeFailure("failing transport always fails")
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
    override class func canInit(with request: URLRequest) -> Bool {
        request.url?.host == "timeout.example.test"
    }

    override class func canonicalRequest(for request: URLRequest) -> URLRequest {
        request
    }

    override func startLoading() {}
    override func stopLoading() {}
}

func pollUntil(
    client: ReaderCoreClient,
    requestId: UInt64,
    timeout: TimeInterval = 5
) throws -> ReaderCoreEvent {
    let deadline = Date().addingTimeInterval(timeout)
    while Date() < deadline {
        if let result = client.pollEvent(requestId: requestId) {
            return try result.get()
        }
    }
    throw SmokeFailure("timed out polling requestId \(requestId)")
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

        let info = try client.coreInfo(requestId: 100, timeout: 5)
        guard info.type == "result", let infoData = info.data else {
            throw SmokeFailure("core.info did not return a result event")
        }
        guard (infoData["abiVersion"] as? NSNumber)?.uint32Value == ReaderCoreRuntime.abiVersion else {
            throw SmokeFailure("core.info ABI version mismatch")
        }

        let ping = try client.ping(requestId: 101, timeout: 5)
        guard ping.type == "result", (ping.data?["pong"] as? Bool) == true else {
            throw SmokeFailure("runtime.ping missing pong=true")
        }

        try client.send(method: "core.info", requestId: 102)
        let polled = try pollUntil(client: client, requestId: 102)
        guard polled.type == "result" else {
            throw SmokeFailure("pollEvent did not observe a result event")
        }
        guard client.pollEvent(requestId: 102) == nil else {
            throw SmokeFailure("pollEvent returned a consumed event")
        }

        try client.send(
            method: "runtime.hostSmoke",
            requestId: 150,
            params: [
                "capability": "host.smoke.echo",
                "params": ["hello": "world"],
            ]
        )
        let hostRequest = try pollUntil(client: client, requestId: 150)
        guard hostRequest.isHostRequest, hostRequest.capability == "host.smoke.echo" else {
            throw SmokeFailure("runtime.hostSmoke did not emit expected host.request")
        }
        guard let operationId = hostRequest.operationId, operationId > 0 else {
            throw SmokeFailure("runtime.hostSmoke operationId missing")
        }
        _ = try client.sendHostComplete(
            operationId: operationId,
            result: ["echoed": true],
            requestId: 151
        )
        let manualResult = try pollUntil(client: client, requestId: 150)
        guard manualResult.type == "result", (manualResult.data?["echoed"] as? Bool) == true else {
            throw SmokeFailure("manual host.complete did not resume original request")
        }

        try client.send(
            method: "runtime.hostSmoke",
            requestId: 1001,
            params: [
                "capability": "host.smoke.echo",
                "params": ["collision": true],
            ]
        )
        let collisionHostRequest = try pollUntil(client: client, requestId: 1001)
        guard let collisionOperationId = collisionHostRequest.operationId, collisionOperationId > 0 else {
            throw SmokeFailure("collision smoke operationId missing")
        }
        _ = try client.sendHostComplete(
            operationId: collisionOperationId,
            result: ["collision": "avoided"]
        )
        let collisionResult = try pollUntil(client: client, requestId: 1001)
        guard collisionResult.type == "result",
              collisionResult.data?["collision"] as? String == "avoided" else {
            throw SmokeFailure("internal host.complete command id collided")
        }

        try client.send(
            method: "runtime.hostSmoke",
            requestId: 175,
            params: [
                "capability": "host.smoke.echo",
                "params": ["cancel": true],
            ]
        )
        let cancelHostRequest = try pollUntil(client: client, requestId: 175)
        guard cancelHostRequest.isHostRequest else {
            throw SmokeFailure("cancel smoke did not produce host.request")
        }
        try client.cancel(requestId: 175)
        let cancelledEvent = try pollUntil(client: client, requestId: 175)
        guard cancelledEvent.type == "error",
              cancelledEvent.coreError?.code == "CANCELLED" else {
            throw SmokeFailure("cancel did not surface CANCELLED")
        }

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
        guard SmokeURLProtocol.lastRequest?.httpMethod == "POST",
              SmokeURLProtocol.lastRequest?.value(forHTTPHeaderField: "Accept") == "application/json",
              urlSessionResponse.status == 202,
              urlSessionResponse.headers["X-Smoke"] == "url-protocol",
              urlSessionResponse.body == "{\"ok\":true}" else {
            throw SmokeFailure("URLSessionHostTransport mapping failed")
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
        guard search.type == "result",
              let books = search.data?["books"] as? [Any],
              books.count == 1 else {
            throw SmokeFailure("book.search did not return one book")
        }
        guard let captured = stub.lastRequest, captured.operationId > 0 else {
            throw SmokeFailure("host transport was not invoked")
        }

        let errorClient = try ReaderCoreClient(hostTransport: FailingHostTransport())
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
            didThrow = !coreError.message.isEmpty
        }
        guard didThrow else {
            throw SmokeFailure("transport failure did not surface as a core error")
        }

        var unknownError: ReaderCoreCoreError?
        do {
            _ = try client.request(method: "no.such.method", requestId: 202, params: [:], timeout: 5)
        } catch ReaderCoreClientError.coreError(let coreError) {
            unknownError = coreError
        }
        guard let unknownError, unknownError.code == "UNKNOWN_METHOD" else {
            throw SmokeFailure("unknown method did not surface UNKNOWN_METHOD")
        }

        let rawRuntime = try ReaderCoreRuntime(onEvent: { _ in })
        defer { rawRuntime.destroy() }
        do {
            try rawRuntime.send(jsonString: "{not valid json")
            throw SmokeFailure("malformed JSON send did not fail")
        } catch ReaderCoreClientError.sendFailed(let status) {
            guard status != 0 else {
                throw SmokeFailure("malformed JSON returned zero status")
            }
        }

        print("swift client smoke passed")
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

if (( swift_only == 1 )); then
  typecheck_wrapper_with_repo_headers
  run_macos_swift_smoke "$host_lib"
  echo "swift-only wrapper checks passed using existing $host_lib"
  exit 0
fi

if ! ./scripts/build-ios-xcframework.sh; then
  echo "full iOS Swift wrapper gate stopped while building the XCFramework" >&2
  echo "wrapper-only validation can be run explicitly with:" >&2
  echo "  bash ./scripts/check-ios-swift-wrapper.sh --swift-only" >&2
  exit 1
fi

headers="target/ios/ReaderCore.xcframework/ios-arm64-simulator/Headers"
typecheck_wrapper "$headers"

if ! cargo build -p reader-ffi; then
  echo "full iOS Swift wrapper gate stopped while building the host static library" >&2
  echo "wrapper-only validation can be run explicitly with:" >&2
  echo "  bash ./scripts/check-ios-swift-wrapper.sh --swift-only" >&2
  exit 1
fi

run_macos_swift_smoke "$host_lib"
