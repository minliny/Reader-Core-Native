package com.reader.core;

import java.io.Closeable;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.TimeUnit;

public final class ReaderCoreRuntime implements Closeable {
    private static final int PROTOCOL_VERSION = 1;
    private static final long FIRST_GENERATED_REQUEST_ID = 1000L;

    private long handle;
    private long nextRequestId = FIRST_GENERATED_REQUEST_ID;

    public ReaderCoreRuntime() {
        this("{}");
    }

    public ReaderCoreRuntime(String configJson) {
        byte[] configBytes = utf8(configJson == null ? "{}" : configJson);
        long created = NativeCoreBridge.create(configBytes);
        if (created == 0) {
            throw new ReaderCoreException("rc_runtime_create failed");
        }
        this.handle = created;
    }

    public static int abiVersion() {
        return NativeCoreBridge.abiVersion();
    }

    public static String pingSmoke() {
        try (ReaderCoreRuntime runtime = new ReaderCoreRuntime("{}")) {
            long requestId = 42L;
            runtime.sendCommand("runtime.ping", requestId, "{}");
            String event = runtime.pollEvent(1, TimeUnit.SECONDS);
            if (event == null) {
                throw new ReaderCoreException("runtime.ping timed out");
            }
            return event;
        }
    }

    public synchronized void send(String commandJson) {
        long runtimeHandle = requireHandle();
        int status = NativeCoreBridge.send(runtimeHandle, utf8(commandJson));
        if (status != 0) {
            throw new ReaderCoreException(
                    status,
                    "rc_runtime_send failed with status " + status);
        }
    }

    public synchronized long sendCommand(String method, String paramsJson) {
        long requestId = nextRequestId++;
        sendCommand(method, requestId, paramsJson);
        return requestId;
    }

    public synchronized void sendCommand(String method, long requestId, String paramsJson) {
        String commandJson = "{\"protocolVersion\":" + PROTOCOL_VERSION
                + ",\"requestId\":" + requestId
                + ",\"method\":\"" + escapeJson(method) + "\""
                + ",\"params\":" + objectOrEmpty(paramsJson)
                + "}";
        send(commandJson);
    }

    public synchronized int cancel(long requestId) {
        long runtimeHandle = requireHandle();
        int status = NativeCoreBridge.cancel(runtimeHandle, requestId);
        if (status != 0) {
            throw new ReaderCoreException(
                    status,
                    "rc_runtime_cancel failed with status " + status);
        }
        return status;
    }

    public synchronized byte[] pollEventBytes(long timeout, TimeUnit unit) {
        long runtimeHandle = requireHandle();
        long timeoutMillis = unit == null ? timeout : unit.toMillis(timeout);
        if (timeoutMillis < 0) {
            timeoutMillis = 0;
        }
        return NativeCoreBridge.pollEvent(runtimeHandle, timeoutMillis);
    }

    public synchronized String pollEvent(long timeout, TimeUnit unit) {
        byte[] bytes = pollEventBytes(timeout, unit);
        if (bytes == null) {
            return null;
        }
        return new String(bytes, StandardCharsets.UTF_8);
    }

    public synchronized long sendHostComplete(long operationId, String resultJson) {
        long requestId = nextRequestId++;
        sendHostComplete(operationId, resultJson, requestId);
        return requestId;
    }

    public synchronized void sendHostComplete(long operationId, String resultJson, long requestId) {
        String paramsJson = "{\"operationId\":" + operationId
                + ",\"result\":" + objectOrEmpty(resultJson)
                + "}";
        sendCommand("host.complete", requestId, paramsJson);
    }

    public synchronized long sendHostError(
            long operationId,
            String code,
            String message,
            boolean retryable) {
        long requestId = nextRequestId++;
        sendHostError(operationId, code, message, retryable, requestId);
        return requestId;
    }

    public synchronized void sendHostError(
            long operationId,
            String code,
            String message,
            boolean retryable,
            long requestId) {
        String paramsJson = "{\"operationId\":" + operationId
                + ",\"error\":{\"code\":\"" + escapeJson(code) + "\""
                + ",\"message\":\"" + escapeJson(message) + "\""
                + ",\"retryable\":" + retryable
                + "}}";
        sendCommand("host.error", requestId, paramsJson);
    }

    @Override
    public synchronized void close() {
        if (handle != 0) {
            NativeCoreBridge.destroy(handle);
            handle = 0;
        }
    }

    private long requireHandle() {
        if (handle == 0) {
            throw new ReaderCoreException("ReaderCore runtime is closed");
        }
        return handle;
    }

    private static byte[] utf8(String value) {
        return (value == null ? "" : value).getBytes(StandardCharsets.UTF_8);
    }

    private static String objectOrEmpty(String json) {
        if (json == null || json.trim().isEmpty()) {
            return "{}";
        }
        return json.trim();
    }

    private static String escapeJson(String value) {
        if (value == null) {
            return "";
        }
        StringBuilder builder = new StringBuilder(value.length() + 16);
        for (int index = 0; index < value.length(); index++) {
            char ch = value.charAt(index);
            switch (ch) {
                case '"':
                    builder.append("\\\"");
                    break;
                case '\\':
                    builder.append("\\\\");
                    break;
                case '\b':
                    builder.append("\\b");
                    break;
                case '\f':
                    builder.append("\\f");
                    break;
                case '\n':
                    builder.append("\\n");
                    break;
                case '\r':
                    builder.append("\\r");
                    break;
                case '\t':
                    builder.append("\\t");
                    break;
                default:
                    if (ch < 0x20) {
                        builder.append(String.format("\\u%04x", (int) ch));
                    } else {
                        builder.append(ch);
                    }
                    break;
            }
        }
        return builder.toString();
    }
}
