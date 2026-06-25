package com.reader.core

/**
 * Receives every Core → platform event on a Core-owned background thread.
 *
 * Implementations MUST be thread-safe: [onEvent] is invoked from a non-UI
 * thread and may be invoked concurrently with [NativeCoreBridge.runtimeSend].
 *
 * Event JSON shapes (see protocol/reader-event.schema.json):
 *  - result:       {"protocolVersion":1,"requestId":..,"type":"result","data":{..}}
 *  - error:        {"protocolVersion":1,"requestId":..,"type":"error","error":{code,message,retryable}}
 *  - host.request: {"protocolVersion":1,"requestId":..,"type":"host.request","operationId":..,"capability":"..","params":{..}}
 *
 * For a `host.request`, answer by sending a `host.complete` or `host.error`
 * command back through [NativeCoreBridge.runtimeSend].
 */
interface ReaderEventListener {
    fun onEvent(eventJson: String)
}
