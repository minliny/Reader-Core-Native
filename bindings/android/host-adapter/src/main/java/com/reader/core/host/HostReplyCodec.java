package com.reader.core.host;

import java.util.Map;

/**
 * Encodes {@link HostReply} values into Core command JSON for
 * {@code host.complete} / {@code host.error}.
 *
 * <p>Output shape matches the {@code protocol/fixtures/conformance/host/}
 * reply fixtures exactly (canonical compact form):
 * <pre>
 * {"protocolVersion":1,"requestId":R,"method":"host.complete",
 *  "params":{"operationId":O,"result":{...}}}
 * {"protocolVersion":1,"requestId":R,"method":"host.error",
 *  "params":{"operationId":O,"error":{"code":"..","message":"..","retryable":b}}}
 * </pre>
 *
 * <p>The encoded command is what the host sends to Core via the existing
 * {@code ReaderCoreRuntime.send} / {@code rc_runtime_send} surface. This codec
 * adds no C ABI symbols.
 */
public final class HostReplyCodec {

    private static final int PROTOCOL_VERSION = 1;

    private HostReplyCodec() {}

    public static String encodeComplete(long requestId, long operationId, String resultJson) {
        requireOperationId(operationId);
        String result = canonicalObject(resultJson);
        return "{\"protocolVersion\":" + PROTOCOL_VERSION
                + ",\"requestId\":" + requestId
                + ",\"method\":\"host.complete\""
                + ",\"params\":{\"operationId\":" + operationId
                + ",\"result\":" + result + "}}";
    }

    public static String encodeError(long requestId, long operationId,
                                     String code, String message, boolean retryable) {
        requireOperationId(operationId);
        if (code == null || code.isEmpty()) {
            throw new IllegalArgumentException("error code required");
        }
        return "{\"protocolVersion\":" + PROTOCOL_VERSION
                + ",\"requestId\":" + requestId
                + ",\"method\":\"host.error\""
                + ",\"params\":{\"operationId\":" + operationId
                + ",\"error\":{\"code\":" + Json.stringify(code)
                + ",\"message\":" + Json.stringify(message == null ? "" : message)
                + ",\"retryable\":" + retryable + "}}}";
    }

    /** Encode a reply using a caller-chosen requestId for the outbound command. */
    public static String encode(long requestId, HostReply reply, long operationId) {
        if (reply == null) {
            throw new IllegalArgumentException("reply required");
        }
        if (reply.isComplete()) {
            return encodeComplete(requestId, operationId, ((HostReply.Complete) reply).resultJson());
        }
        HostReply.Error err = (HostReply.Error) reply;
        return encodeError(requestId, operationId, err.code(), err.message(), err.retryable());
    }

    private static void requireOperationId(long operationId) {
        if (operationId < 1) {
            throw new IllegalArgumentException("operationId must be >= 1, got " + operationId);
        }
    }

    private static String canonicalObject(String json) {
        if (json == null || json.trim().isEmpty()) {
            return "{}";
        }
        Object parsed = Json.parse(json.trim());
        if (!(parsed instanceof Map)) {
            throw new IllegalArgumentException("result must be a JSON object");
        }
        return Json.stringify(parsed);
    }
}
