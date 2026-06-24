import Foundation
import ReaderCore

public enum ReaderCoreClientError: Error, Equatable {
    case createFailed(Int32)
    case sendFailed(Int32)
    case cancelFailed(Int32)
    case runtimeDestroyed
    case invalidCommandJSON
    case invalidEventJSON
    case requestTimedOut(UInt64)
    case coreError(Data)
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
}

public final class ReaderCoreRuntime {
    public static var abiVersion: UInt32 {
        rc_abi_version()
    }

    private var handle: OpaquePointer?
    private let sinkContext: UnsafeMutableRawPointer

    public init(configJSON: Data = Data(), onEvent: @escaping (Data) -> Void) throws {
        let sink = ReaderCoreEventSink(onEvent: onEvent)
        self.sinkContext = Unmanaged.passRetained(sink).toOpaque()

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
            Unmanaged<ReaderCoreEventSink>.fromOpaque(sinkContext).release()
            throw ReaderCoreClientError.createFailed(status)
        }

        self.handle = runtime
    }

    deinit {
        if let handle {
            rc_runtime_destroy(handle)
        }
        Unmanaged<ReaderCoreEventSink>.fromOpaque(sinkContext).release()
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

    public init(configJSON: Data = Data()) throws {
        let eventBuffer = ReaderCoreEventBuffer()
        self.eventBuffer = eventBuffer
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

    public func destroy() {
        runtime.destroy()
    }

    private func request(
        method: String,
        requestId: UInt64,
        params: [String: Any] = [:],
        timeout: TimeInterval
    ) throws -> ReaderCoreEvent {
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
        let event = try eventBuffer.wait(requestId: requestId, timeout: timeout)
        if event.type == "error" {
            throw ReaderCoreClientError.coreError(event.rawData)
        }
        return event
    }
}
