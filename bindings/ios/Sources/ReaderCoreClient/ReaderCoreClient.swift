import Foundation
import ReaderCore

public enum ReaderCoreClientError: Error, Equatable {
    case createFailed(Int32)
    case sendFailed(Int32)
    case cancelFailed(Int32)
    case runtimeDestroyed
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
