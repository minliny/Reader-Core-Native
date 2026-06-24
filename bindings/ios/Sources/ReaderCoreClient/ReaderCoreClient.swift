import Foundation
import ReaderCore

public struct ReaderCoreCoreError: Error, Equatable {
    public let code: String
    public let message: String
    public let retryable: Bool
    public let rawData: Data

    public init(code: String, message: String, retryable: Bool, rawData: Data) {
        self.code = code
        self.message = message
        self.retryable = retryable
        self.rawData = rawData
    }
}

public enum ReaderCoreClientError: Error, Equatable {
    case createFailed(Int32)
    case sendFailed(Int32)
    case cancelFailed(Int32)
    case runtimeDestroyed
    case invalidCommandJSON
    case invalidEventJSON
    case requestTimedOut(UInt64)
    case missingHostTransport
    case hostTransportFailed(String)
    case coreError(ReaderCoreCoreError)
}

public struct ReaderCoreEvent {
    public let rawData: Data
    public let object: [String: Any]

    public init(data: Data) throws {
        let value = try? JSONSerialization.jsonObject(with: data)
        guard let object = value as? [String: Any] else {
            throw ReaderCoreClientError.invalidEventJSON
        }

        self.rawData = data
        self.object = object
    }

    public var type: String {
        object["type"] as? String ?? ""
    }

    public var requestId: UInt64? {
        Self.uint64Value(object["requestId"])
    }

    public var data: [String: Any]? {
        object["data"] as? [String: Any]
    }

    public var error: [String: Any]? {
        object["error"] as? [String: Any]
    }

    public var isHostRequest: Bool {
        type == "host.request"
    }

    public var operationId: UInt64? {
        Self.uint64Value(object["operationId"])
    }

    public var capability: String? {
        object["capability"] as? String
    }

    public var hostRequestParams: [String: Any]? {
        object["params"] as? [String: Any]
    }

    public var coreError: ReaderCoreCoreError? {
        guard type == "error" else { return nil }
        let err = error ?? [:]
        return ReaderCoreCoreError(
            code: err["code"] as? String ?? "INTERNAL",
            message: err["message"] as? String ?? "",
            retryable: err["retryable"] as? Bool ?? false,
            rawData: rawData
        )
    }

    private static func uint64Value(_ value: Any?) -> UInt64? {
        if let value = value as? UInt64 {
            return value
        }
        if let value = value as? NSNumber {
            return value.uint64Value
        }
        if let value = value as? String {
            return UInt64(value)
        }
        return nil
    }
}

public struct ReaderCoreHostRequest {
    public let operationId: UInt64
    public let capability: String
    public let rawParams: [String: Any]

    public init(operationId: UInt64, capability: String, rawParams: [String: Any]) {
        self.operationId = operationId
        self.capability = capability
        self.rawParams = rawParams
    }

    public var url: String? {
        rawParams["url"] as? String
    }

    public var method: String? {
        rawParams["method"] as? String
    }

    public var headers: [String: String] {
        guard let object = rawParams["headers"] as? [String: Any] else {
            return [:]
        }
        var headers: [String: String] = [:]
        for (key, value) in object {
            headers[key] = value as? String ?? "\(value)"
        }
        return headers
    }

    public var body: String? {
        rawParams["body"] as? String
    }
}

public struct ReaderCoreHostResponse {
    public let status: Int
    public let headers: [String: String]
    public let body: String

    public init(status: Int = 200, headers: [String: String] = [:], body: String) {
        self.status = status
        self.headers = headers
        self.body = body
    }

    public var resultObject: [String: Any] {
        var object: [String: Any] = ["status": status, "body": body]
        if !headers.isEmpty {
            object["headers"] = headers
        }
        return object
    }
}

public protocol ReaderCoreHostTransport {
    func perform(_ request: ReaderCoreHostRequest) throws -> ReaderCoreHostResponse
}

public final class URLSessionHostTransport: ReaderCoreHostTransport {
    public let session: URLSession
    public let timeout: TimeInterval

    public init(session: URLSession = .shared, timeout: TimeInterval = 30) {
        self.session = session
        self.timeout = timeout
    }

    public func perform(_ request: ReaderCoreHostRequest) throws -> ReaderCoreHostResponse {
        guard let urlString = request.url, !urlString.isEmpty else {
            throw ReaderCoreClientError.hostTransportFailed("http.execute request missing url")
        }
        guard let url = URL(string: urlString) else {
            throw ReaderCoreClientError.hostTransportFailed("http.execute request url is invalid")
        }

        var urlRequest = URLRequest(url: url)
        urlRequest.httpMethod = request.method ?? "GET"
        for (field, value) in request.headers {
            urlRequest.setValue(value, forHTTPHeaderField: field)
        }
        if let body = request.body, !body.isEmpty {
            urlRequest.httpBody = body.data(using: .utf8)
        }

        let semaphore = DispatchSemaphore(value: 0)
        var captured: Result<ReaderCoreHostResponse, Error>?

        let task = session.dataTask(with: urlRequest) { data, response, error in
            if let error {
                captured = .failure(error)
            } else if let httpResponse = response as? HTTPURLResponse {
                let bodyString = data.flatMap { String(data: $0, encoding: .utf8) } ?? ""
                var headers: [String: String] = [:]
                for (key, value) in httpResponse.allHeaderFields {
                    if let key = key as? String {
                        headers[key] = value as? String ?? "\(value)"
                    }
                }
                captured = .success(ReaderCoreHostResponse(
                    status: httpResponse.statusCode,
                    headers: headers,
                    body: bodyString
                ))
            } else {
                captured = .failure(ReaderCoreClientError.hostTransportFailed("non-http response"))
            }
            semaphore.signal()
        }

        task.resume()
        if semaphore.wait(timeout: .now() + timeout) == .timedOut {
            task.cancel()
            throw ReaderCoreClientError.hostTransportFailed("http.execute request timed out")
        }

        guard let result = captured else {
            throw ReaderCoreClientError.hostTransportFailed("urlsession produced no result")
        }
        return try result.get()
    }
}

private final class ReaderCoreEventSink {
    let onEvent: (Data) -> Void

    init(onEvent: @escaping (Data) -> Void) {
        self.onEvent = onEvent
    }
}

private let readerCoreEventCallback: rc_event_callback = { context, bytes, length in
    guard let context, let bytes else {
        return
    }

    let sink = Unmanaged<ReaderCoreEventSink>.fromOpaque(context).takeUnretainedValue()
    sink.onEvent(Data(bytes: bytes, count: length))
}

private final class ReaderCoreEventBuffer {
    private let condition = NSCondition()
    private var events: [Result<ReaderCoreEvent, ReaderCoreClientError>] = []

    func append(_ data: Data) {
        let event: Result<ReaderCoreEvent, ReaderCoreClientError>
        do {
            event = .success(try ReaderCoreEvent(data: data))
        } catch let error as ReaderCoreClientError {
            event = .failure(error)
        } catch {
            event = .failure(.invalidEventJSON)
        }

        condition.lock()
        events.append(event)
        condition.broadcast()
        condition.unlock()
    }

    func wait(requestId: UInt64, timeout: TimeInterval) throws -> ReaderCoreEvent {
        let deadline = Date().addingTimeInterval(timeout)

        condition.lock()
        defer { condition.unlock() }

        while true {
            for (index, event) in events.enumerated() {
                switch event {
                case .success(let event) where event.requestId == requestId:
                    events.remove(at: index)
                    return event
                case .failure(let error):
                    events.remove(at: index)
                    throw error
                default:
                    continue
                }
            }

            let remaining = deadline.timeIntervalSinceNow
            guard remaining > 0 else {
                throw ReaderCoreClientError.requestTimedOut(requestId)
            }

            condition.wait(until: deadline)
        }
    }

    func poll(requestId: UInt64) -> Result<ReaderCoreEvent, ReaderCoreClientError>? {
        condition.lock()
        defer { condition.unlock() }

        for (index, event) in events.enumerated() {
            switch event {
            case .success(let event) where event.requestId == requestId:
                events.remove(at: index)
                return .success(event)
            case .failure(let error):
                events.remove(at: index)
                return .failure(error)
            default:
                continue
            }
        }
        return nil
    }
}

public final class ReaderCoreRuntime {
    public static var abiVersion: UInt32 {
        rc_abi_version()
    }

    private var handle: OpaquePointer?
    private var sinkContext: UnsafeMutableRawPointer?

    public init(configJSON: Data = Data(), onEvent: @escaping (Data) -> Void) throws {
        self.sinkContext = nil

        let sink = ReaderCoreEventSink(onEvent: onEvent)
        let sinkContext = Unmanaged.passRetained(sink).toOpaque()
        self.sinkContext = sinkContext

        var runtime: OpaquePointer?
        let status = configJSON.withUnsafeBytes { rawBuffer in
            rc_runtime_create(
                rawBuffer.bindMemory(to: UInt8.self).baseAddress,
                configJSON.count,
                readerCoreEventCallback,
                sinkContext,
                &runtime
            )
        }

        guard status == 0, let runtime else {
            self.sinkContext = nil
            Unmanaged<ReaderCoreEventSink>.fromOpaque(sinkContext).release()
            throw ReaderCoreClientError.createFailed(status)
        }

        self.handle = runtime
    }

    deinit {
        if let handle {
            rc_runtime_destroy(handle)
        }
        if let sinkContext {
            Unmanaged<ReaderCoreEventSink>.fromOpaque(sinkContext).release()
        }
    }

    public func send(json: Data) throws {
        guard let handle else {
            throw ReaderCoreClientError.runtimeDestroyed
        }

        let status = json.withUnsafeBytes { rawBuffer in
            rc_runtime_send(
                handle,
                rawBuffer.bindMemory(to: UInt8.self).baseAddress,
                json.count
            )
        }

        guard status == 0 else {
            throw ReaderCoreClientError.sendFailed(status)
        }
    }

    public func send(jsonString: String) throws {
        guard let data = jsonString.data(using: .utf8) else {
            throw ReaderCoreClientError.sendFailed(-1)
        }
        try send(json: data)
    }

    public func cancel(requestId: UInt64) throws {
        guard let handle else {
            throw ReaderCoreClientError.runtimeDestroyed
        }

        let status = rc_runtime_cancel(handle, requestId)
        guard status == 0 else {
            throw ReaderCoreClientError.cancelFailed(status)
        }
    }

    public func destroy() {
        if let handle {
            rc_runtime_destroy(handle)
            self.handle = nil
        }
    }
}

public final class ReaderCoreClient {
    public static let protocolVersion: UInt32 = 1

    private let eventBuffer: ReaderCoreEventBuffer
    private let runtime: ReaderCoreRuntime
    private let hostTransport: ReaderCoreHostTransport?

    private let commandIdLock = NSLock()
    private var commandIdCounter: UInt64 = 9_000_000_000_000_000

    public init(configJSON: Data = Data(), hostTransport: ReaderCoreHostTransport? = nil) throws {
        let eventBuffer = ReaderCoreEventBuffer()
        self.eventBuffer = eventBuffer
        self.hostTransport = hostTransport
        self.runtime = try ReaderCoreRuntime(configJSON: configJSON) { data in
            eventBuffer.append(data)
        }
    }

    @discardableResult
    public func coreInfo(requestId: UInt64 = 1, timeout: TimeInterval = 2) throws -> ReaderCoreEvent {
        try request(method: "core.info", requestId: requestId, timeout: timeout)
    }

    @discardableResult
    public func ping(requestId: UInt64 = 2, timeout: TimeInterval = 2) throws -> ReaderCoreEvent {
        try request(method: "runtime.ping", requestId: requestId, timeout: timeout)
    }

    @discardableResult
    public func send(
        method: String,
        requestId: UInt64,
        params: [String: Any] = [:]
    ) throws -> UInt64 {
        try sendCommand(method: method, requestId: requestId, params: params)
        return requestId
    }

    @discardableResult
    public func request(
        method: String,
        requestId: UInt64,
        params: [String: Any] = [:],
        timeout: TimeInterval = 5
    ) throws -> ReaderCoreEvent {
        try sendCommand(method: method, requestId: requestId, params: params)
        return try waitForResolved(requestId: requestId, timeout: timeout)
    }

    @discardableResult
    public func pollEvent(requestId: UInt64) -> Result<ReaderCoreEvent, ReaderCoreClientError>? {
        eventBuffer.poll(requestId: requestId)
    }

    public func cancel(requestId: UInt64) throws {
        try runtime.cancel(requestId: requestId)
    }

    @discardableResult
    public func sendHostComplete(
        operationId: UInt64,
        result: [String: Any],
        requestId: UInt64? = nil
    ) throws -> UInt64 {
        let commandId = requestId ?? nextCommandId()
        try sendCommand(
            method: "host.complete",
            requestId: commandId,
            params: [
                "operationId": NSNumber(value: operationId),
                "result": result,
            ]
        )
        return commandId
    }

    @discardableResult
    public func sendHostError(
        operationId: UInt64,
        code: String,
        message: String,
        retryable: Bool,
        requestId: UInt64? = nil
    ) throws -> UInt64 {
        let commandId = requestId ?? nextCommandId()
        try sendCommand(
            method: "host.error",
            requestId: commandId,
            params: [
                "operationId": NSNumber(value: operationId),
                "error": [
                    "code": code,
                    "message": message,
                    "retryable": retryable,
                ],
            ]
        )
        return commandId
    }

    public func destroy() {
        runtime.destroy()
    }

    private func sendCommand(method: String, requestId: UInt64, params: [String: Any]) throws {
        let command: [String: Any] = [
            "protocolVersion": NSNumber(value: Self.protocolVersion),
            "requestId": NSNumber(value: requestId),
            "method": method,
            "params": params,
        ]

        guard JSONSerialization.isValidJSONObject(command) else {
            throw ReaderCoreClientError.invalidCommandJSON
        }

        let json: Data
        do {
            json = try JSONSerialization.data(withJSONObject: command)
        } catch {
            throw ReaderCoreClientError.invalidCommandJSON
        }

        try runtime.send(json: json)
    }

    private func waitForResolved(requestId: UInt64, timeout: TimeInterval) throws -> ReaderCoreEvent {
        let deadline = Date().addingTimeInterval(timeout)

        while true {
            let remaining = max(0, deadline.timeIntervalSinceNow)
            let event = try eventBuffer.wait(requestId: requestId, timeout: remaining)

            switch event.type {
            case "result":
                return event
            case "error":
                throw ReaderCoreClientError.coreError(event.coreError ?? ReaderCoreCoreError(
                    code: "INTERNAL",
                    message: "",
                    retryable: false,
                    rawData: event.rawData
                ))
            case "host.request":
                guard let transport = hostTransport else {
                    throw ReaderCoreClientError.missingHostTransport
                }
                let hostRequest = try makeHostRequest(from: event)
                do {
                    let response = try transport.perform(hostRequest)
                    _ = try sendHostComplete(
                        operationId: hostRequest.operationId,
                        result: response.resultObject
                    )
                } catch let error as ReaderCoreClientError {
                    _ = try sendHostError(
                        operationId: hostRequest.operationId,
                        code: "INTERNAL",
                        message: "\(error)",
                        retryable: false
                    )
                } catch {
                    _ = try sendHostError(
                        operationId: hostRequest.operationId,
                        code: "INTERNAL",
                        message: "\(error)",
                        retryable: false
                    )
                }
                continue
            default:
                throw ReaderCoreClientError.invalidEventJSON
            }
        }
    }

    private func makeHostRequest(from event: ReaderCoreEvent) throws -> ReaderCoreHostRequest {
        guard event.isHostRequest else {
            throw ReaderCoreClientError.invalidEventJSON
        }
        return ReaderCoreHostRequest(
            operationId: event.operationId ?? 0,
            capability: event.capability ?? "",
            rawParams: event.hostRequestParams ?? [:]
        )
    }

    private func nextCommandId() -> UInt64 {
        commandIdLock.lock()
        defer { commandIdLock.unlock() }
        commandIdCounter += 1
        return commandIdCounter
    }
}
