// iOS host adapter ShellSmokeTest.
//
// Runs against the real Rust Core (target/debug/libreader_core.a) on macOS host.
// Each case prints a partitioned tag:
//   [core]      — exercised through the Rust Core via the C ABI / JSON protocol.
//   [app-side]  — exercised by the iOS Swift adapter (wrapper + URLSessionHostTransport
//                  + host.request/complete/error routing).
//
// This is wrapper/host smoke, NOT iOS App/device proof. A green run only proves the
// adapter compiles, links, and drives the host Core build. See README.md.

import Foundation

struct SmokeFailure: Error, CustomStringConvertible {
    let description: String
    init(_ description: String) { self.description = description }
}

var corePass = 0
var coreFail = 0
var appPass = 0
var appFail = 0
var failures: [String] = []

func check(_ tag: String, _ name: String, _ ok: Bool, _ detail: String = "") {
    let line = tag.hasPrefix("[core]") ? "[core] \(name)" : "[app-side] \(name)"
    if ok {
        print("\(line): PASS")
        if tag.hasPrefix("[core]") { corePass += 1 } else { appPass += 1 }
    } else {
        print("\(line): FAIL\(detail.isEmpty ? "" : " — \(detail)")")
        if tag.hasPrefix("[core]") { coreFail += 1 } else { appFail += 1 }
        failures.append("\(line)\(detail.isEmpty ? "" : " — \(detail)")")
    }
}

// MARK: - Stubs

final class EchoHostTransport: ReaderCoreHostTransport {
    var lastRequest: ReaderCoreHostRequest?
    func perform(_ request: ReaderCoreHostRequest) throws -> ReaderCoreHostResponse {
        lastRequest = request
        // Echo rawParams back as the result object so the Core resumes the original request.
        return ReaderCoreHostResponse(status: 200, headers: [:], body: "{}")
    }
}

final class FailingHostTransport: ReaderCoreHostTransport {
    func perform(_ request: ReaderCoreHostRequest) throws -> ReaderCoreHostResponse {
        throw SmokeFailure("failing transport always fails")
    }
}

final class BooksHostTransport: ReaderCoreHostTransport {
    var lastRequest: ReaderCoreHostRequest?
    func perform(_ request: ReaderCoreHostRequest) throws -> ReaderCoreHostResponse {
        lastRequest = request
        guard request.capability == "http.execute" else {
            throw SmokeFailure("BooksHostTransport only handles http.execute, got \(request.capability)")
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
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        Self.lastRequest = request
        let response = HTTPURLResponse(
            url: request.url!, statusCode: 202, httpVersion: nil,
            headerFields: ["Content-Type": "application/json", "X-Smoke": "url-protocol"]
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
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {}
    override func stopLoading() {}
}

func pollUntil(client: ReaderCoreClient, requestId: UInt64, timeout: TimeInterval = 5) throws -> ReaderCoreEvent {
    let deadline = Date().addingTimeInterval(timeout)
    while Date() < deadline {
        if let r = client.pollEvent(requestId: requestId) { return try r.get() }
    }
    throw SmokeFailure("timed out polling requestId \(requestId)")
}

// MARK: - Run

@main
struct HostAdapterSmoke {
    static func main() {
        let stub = EchoHostTransport()
        guard let client = try? ReaderCoreClient(hostTransport: stub) else {
            print("[app-side] ReaderCoreClient create: FAIL — init threw")
            exit(1)
        }
        defer { client.destroy() }

        // ---- [core] ABI / protocol surface exposed by the Rust Core ----
        check("[core]", "abi version == 1",
              ReaderCoreRuntime.abiVersion == 1,
              "got \(ReaderCoreRuntime.abiVersion)")

        var info: ReaderCoreEvent?
        do { info = try client.coreInfo(requestId: 100, timeout: 5) }
        catch { check("[core]", "core.info result", false, "\(error)") }
        let infoOk = info?.type == "result"
            && (info?.data?["abiVersion"] as? NSNumber)?.uint32Value == ReaderCoreRuntime.abiVersion
            && (info?.data?["protocolVersion"] as? NSNumber)?.uint32Value == ReaderCoreClient.protocolVersion
        check("[core]", "core.info returns abi+protocol version", infoOk)

        let caps = (info?.data?["capabilities"] as? [Any])?.compactMap { $0 as? String } ?? []
        check("[core]", "core.info advertises host bus capability",
              caps.contains("host.bus.v1") || caps.contains("http.execute"),
              "capabilities=\(caps)")

        let pingOk: Bool
        do {
            let ping = try client.ping(requestId: 101, timeout: 5)
            pingOk = ping.type == "result" && (ping.data?["pong"] as? Bool) == true
        } catch { pingOk = false }
        check("[core]", "runtime.ping pong=true", pingOk)

        // ---- [core] host.request emission + host.complete resume ----
        do {
            try client.send(method: "runtime.hostSmoke", requestId: 150, params: [
                "capability": "host.smoke.echo",
                "params": ["hello": "world"],
            ])
            let hostReq = try pollUntil(client: client, requestId: 150)
            let emitted = hostReq.isHostRequest && hostReq.capability == "host.smoke.echo"
                && (hostReq.operationId ?? 0) > 0
            check("[core]", "Core emits host.request with operationId", emitted)

            if let opId = hostReq.operationId {
                _ = try client.sendHostComplete(operationId: opId, result: ["echoed": true], requestId: 151)
                let resumed = try pollUntil(client: client, requestId: 150)
                check("[core]", "host.complete resumes original request",
                      resumed.type == "result" && (resumed.data?["echoed"] as? Bool) == true)
            }
        } catch {
            check("[core]", "host.request/complete loop", false, "\(error)")
        }

        // ---- [core] cancel surfaces CANCELLED ----
        do {
            try client.send(method: "runtime.hostSmoke", requestId: 175, params: [
                "capability": "host.smoke.echo", "params": ["cancel": true],
            ])
            _ = try pollUntil(client: client, requestId: 175)
            try client.cancel(requestId: 175)
            let ev = try pollUntil(client: client, requestId: 175)
            check("[core]", "cancel surfaces CANCELLED",
                  ev.type == "error" && ev.coreError?.code == "CANCELLED")
        } catch {
            check("[core]", "cancel surfaces CANCELLED", false, "\(error)")
        }

        // ---- [core] unknown method surfaces structured error ----
        do {
            _ = try client.request(method: "no.such.method", requestId: 202, params: [:], timeout: 5)
            check("[core]", "unknown method surfaces UNKNOWN_METHOD", false, "expected throw")
        } catch ReaderCoreClientError.coreError(let coreError) {
            check("[core]", "unknown method surfaces UNKNOWN_METHOD", coreError.code == "UNKNOWN_METHOD",
                  "code=\(coreError.code)")
        } catch {
            check("[core]", "unknown method surfaces UNKNOWN_METHOD", false, "\(error)")
        }

        // ---- [app-side] adapter lifecycle + host request field mapping ----
        check("[app-side]", "ReaderCoreClient create + destroy", true)

        let mapped = ReaderCoreHostRequest(operationId: 7, capability: "http.execute", rawParams: [
            "url": "https://books.example.test/x",
            "method": "GET",
            "headers": ["Accept": "application/json"],
            "body": "{\"q\":\"dune\"}",
        ])
        check("[app-side]", "ReaderCoreHostRequest maps url/method/headers/body",
              mapped.url == "https://books.example.test/x"
              && mapped.method == "GET"
              && mapped.headers["Accept"] == "application/json"
              && mapped.body == "{\"q\":\"dune\"}")

        // ---- [app-side] URLSessionHostTransport success mapping ----
        let cfg = URLSessionConfiguration.ephemeral
        cfg.protocolClasses = [SmokeURLProtocol.self]
        let session = URLSessionHostTransport(session: URLSession(configuration: cfg))
        do {
            let resp = try session.perform(ReaderCoreHostRequest(
                operationId: 176, capability: "http.execute",
                rawParams: [
                    "url": "https://transport.example.test/books",
                    "method": "POST",
                    "headers": ["Accept": "application/json"],
                    "body": "{\"q\":\"dune\"}",
                ]
            ))
            check("[app-side]", "URLSessionHostTransport maps method/headers/status/body",
                  SmokeURLProtocol.lastRequest?.httpMethod == "POST"
                  && SmokeURLProtocol.lastRequest?.value(forHTTPHeaderField: "Accept") == "application/json"
                  && resp.status == 202
                  && resp.headers["X-Smoke"] == "url-protocol"
                  && resp.body == "{\"ok\":true}")
        } catch {
            check("[app-side]", "URLSessionHostTransport maps method/headers/status/body", false, "\(error)")
        }

        // ---- [app-side] URLSessionHostTransport timeout ----
        let tcfg = URLSessionConfiguration.ephemeral
        tcfg.protocolClasses = [HangingURLProtocol.self]
        let timeoutTransport = URLSessionHostTransport(session: URLSession(configuration: tcfg), timeout: 0.05)
        var timedOut = false
        do {
            _ = try timeoutTransport.perform(ReaderCoreHostRequest(
                operationId: 177, capability: "http.execute",
                rawParams: ["url": "https://timeout.example.test/hang"]
            ))
        } catch ReaderCoreClientError.hostTransportFailed(let message) {
            timedOut = message.contains("timed out")
        } catch { timedOut = false }
        check("[app-side]", "URLSessionHostTransport timeout → hostTransportFailed", timedOut)

        // ---- [app-side] transport failure → host.error → core error ----
        let errorClient = try? ReaderCoreClient(hostTransport: FailingHostTransport())
        guard let errorClient = errorClient else {
            check("[app-side]", "failing-transport client create", false, "init threw")
            exit(1)
        }
        defer { errorClient.destroy() }
        do {
            _ = try errorClient.request(method: "book.search", requestId: 201, params: [
                "sourceId": "vtest-src",
                "searchRequest": ["url": "https://books.example.test/search"],
                "source": [
                    "sourceId": "vtest-src", "name": "V", "baseUrl": "https://books.example.test",
                    "rules": ["search": [["kind": "jsonPath", "path": "$.books[*]"]]],
                ],
            ], timeout: 5)
            check("[app-side]", "transport failure surfaces core error", false, "expected throw")
        } catch ReaderCoreClientError.coreError(let coreError) {
            check("[app-side]", "transport failure surfaces core error", !coreError.message.isEmpty,
                  "code=\(coreError.code)")
        } catch {
            check("[app-side]", "transport failure surfaces core error", false, "\(error)")
        }

        // ---- [app-side] pollEvent non-blocking drain + consumed semantics ----
        do {
            try client.send(method: "core.info", requestId: 300)
            let polled = try pollUntil(client: client, requestId: 300)
            check("[app-side]", "pollEvent drains result event", polled.type == "result")
            check("[app-side]", "pollEvent returns nil for consumed event",
                  client.pollEvent(requestId: 300) == nil)
        } catch {
            check("[app-side]", "pollEvent drain + consumed", false, "\(error)")
        }

        // ---- [core] malformed JSON send fails with non-zero status ----
        do {
            let rawRuntime = try ReaderCoreRuntime(onEvent: { _ in })
            defer { rawRuntime.destroy() }
            do {
                try rawRuntime.send(jsonString: "{not valid json")
                check("[core]", "malformed JSON send fails with non-zero status", false, "expected throw")
            } catch ReaderCoreClientError.sendFailed(let status) {
                check("[core]", "malformed JSON send fails with non-zero status", status != 0, "status=\(status)")
            } catch {
                check("[core]", "malformed JSON send fails with non-zero status", false, "\(error)")
            }
        } catch {
            check("[core]", "malformed JSON send fails with non-zero status", false, "rawRuntime init threw: \(error)")
        }

        // ---- [app-side] internal command ID collision avoidance ----
        // host.complete with auto-allocated requestId must not collide with the user's
        // hostSmoke requestId (1001); the original request must still resume.
        do {
            try client.send(method: "runtime.hostSmoke", requestId: 1001, params: [
                "capability": "host.smoke.echo", "params": ["collision": true],
            ])
            let hostReq = try pollUntil(client: client, requestId: 1001)
            guard let opId = hostReq.operationId, opId > 0 else {
                check("[app-side]", "internal command ID collision avoidance", false, "no operationId")
                exit(1)
            }
            _ = try client.sendHostComplete(operationId: opId, result: ["collision": "avoided"])
            let resumed = try pollUntil(client: client, requestId: 1001)
            check("[app-side]", "internal command ID collision avoidance",
                  resumed.type == "result" && (resumed.data?["collision"] as? String) == "avoided")
        } catch {
            check("[app-side]", "internal command ID collision avoidance", false, "\(error)")
        }

        // ---- [app-side] manual host.error resumes original request as error ----
        do {
            try client.send(method: "runtime.hostSmoke", requestId: 1100, params: [
                "capability": "host.smoke.echo", "params": ["fail": true],
            ])
            let hostReq = try pollUntil(client: client, requestId: 1100)
            guard let opId = hostReq.operationId, opId > 0 else {
                check("[app-side]", "manual host.error resumes as error", false, "no operationId")
                exit(1)
            }
            _ = try client.sendHostError(
                operationId: opId, code: "INTERNAL", message: "manual host error",
                retryable: true
            )
            let resumed = try pollUntil(client: client, requestId: 1100)
            check("[app-side]", "manual host.error resumes original request as error",
                  resumed.type == "error" && resumed.coreError?.code == "INTERNAL"
                  && resumed.coreError?.message == "manual host error",
                  "type=\(resumed.type) code=\(resumed.coreError?.code ?? "nil")")
        } catch {
            check("[app-side]", "manual host.error resumes original request as error", false, "\(error)")
        }

        // ---- [app-side] book.search host HTTP loop returns books ----
        let booksStub = BooksHostTransport()
        guard let booksClient = try? ReaderCoreClient(hostTransport: booksStub) else {
            check("[app-side]", "book.search host HTTP loop returns books", false, "client init threw")
            exit(1)
        }
        defer { booksClient.destroy() }
        do {
            let searchSource: [String: Any] = [
                "sourceId": "vtest-src",
                "name": "Vertical Test Source",
                "baseUrl": "https://books.example.test",
                "rules": ["search": [["kind": "jsonPath", "path": "$.books[*]"]]],
            ]
            let search = try booksClient.request(method: "book.search", requestId: 1200, params: [
                "sourceId": "vtest-src",
                "searchRequest": [
                    "url": "https://books.example.test/search?q=dune",
                    "headers": ["Accept": "application/json"],
                ],
                "source": searchSource,
            ], timeout: 5)
            let books = search.data?["books"] as? [Any]
            check("[app-side]", "book.search host HTTP loop returns books",
                  search.type == "result" && books?.count == 1,
                  "type=\(search.type) count=\(books?.count ?? -1)")
            check("[app-side]", "book.search invoked host transport with operationId",
                  (booksStub.lastRequest?.operationId ?? 0) > 0
                  && booksStub.lastRequest?.capability == "http.execute",
                  "opId=\(booksStub.lastRequest?.operationId ?? 0)")
        } catch {
            check("[app-side]", "book.search host HTTP loop returns books", false, "\(error)")
        }

        // ---- [app-side] sendHostError rejects unknown ErrorCode ----
        // Core's host.error deserializes code as the ErrorCode enum; an unknown
        // code would silently break the host.error path. The adapter validates
        // up front and throws invalidHostErrorCode instead.
        do {
            _ = try client.sendHostError(
                operationId: 9999, code: "BOGUS_CODE", message: "x", retryable: false
            )
            check("[app-side]", "sendHostError rejects unknown ErrorCode", false, "expected throw")
        } catch ReaderCoreClientError.invalidHostErrorCode(let code) {
            check("[app-side]", "sendHostError rejects unknown ErrorCode", code == "BOGUS_CODE",
                  "code=\(code)")
        } catch {
            check("[app-side]", "sendHostError rejects unknown ErrorCode", false, "\(error)")
        }

        // ---- summary ----
        print("---")
        print("[core]     pass=\(corePass) fail=\(coreFail)")
        print("[app-side] pass=\(appPass) fail=\(appFail)")
        if !failures.isEmpty {
            print("FAILURES:")
            failures.forEach { print("  - \($0)") }
            exit(1)
        }
        print("host adapter shell smoke passed")
    }
}
