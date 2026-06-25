package com.reader.core.host;

import java.util.Map;
import java.util.regex.Pattern;

/**
 * A parsed Core {@code host.request} event.
 *
 * <p>Wire shape (per {@code protocol/reader-event.schema.json}):
 * <pre>
 * {
 *   "protocolVersion": 1,
 *   "requestId": &lt;int, original Core command blocked on this op&gt;,
 *   "type": "host.request",
 *   "operationId": &lt;int, &gt;= 1&gt;,
 *   "capability": "&lt;dotted&gt;",
 *   "params": { ... }
 * }
 * </pre>
 *
 * <p>This is the host-side adapter's input. The adapter does not touch the C
 * ABI to obtain it; the host app obtains the event bytes via the existing
 * {@code ReaderCoreRuntime} poll/callback surface and hands the parsed request
 * to {@link HostAdapter}.
 */
public final class HostRequest {

    private static final Pattern CAPABILITY = Pattern.compile("^[^\\s.]+(\\.[^\\s.]+)+$");

    private final long requestId;
    private final long operationId;
    private final String capability;
    private final String paramsJson;

    public HostRequest(long requestId, long operationId, String capability, String paramsJson) {
        if (operationId < 1) {
            throw new IllegalArgumentException("operationId must be >= 1, got " + operationId);
        }
        if (capability == null || !CAPABILITY.matcher(capability).matches()) {
            throw new IllegalArgumentException("invalid capability: " + capability);
        }
        this.requestId = requestId;
        this.operationId = operationId;
        this.capability = capability;
        this.paramsJson = paramsJson == null ? "{}" : paramsJson;
    }

    public long requestId() {
        return requestId;
    }

    public long operationId() {
        return operationId;
    }

    public String capability() {
        return capability;
    }

    /** Canonical JSON string of the request params object. */
    public String paramsJson() {
        return paramsJson;
    }

    /** Parse and validate a {@code host.request} event JSON string. */
    @SuppressWarnings("unchecked")
    public static HostRequest parse(String eventJson) {
        Object root;
        try {
            root = Json.parse(eventJson);
        } catch (Json.JsonException e) {
            throw new IllegalArgumentException("malformed host.request event: " + e.getMessage());
        }
        if (!(root instanceof Map)) {
            throw new IllegalArgumentException("host.request event must be a JSON object");
        }
        Map<String, Object> m = (Map<String, Object>) root;
        Object type = m.get("type");
        if (!"host.request".equals(type)) {
            throw new IllegalArgumentException("not a host.request event: type=" + type);
        }
        long operationId = asLong(m.get("operationId"), "operationId");
        long requestId = asLong(m.get("requestId"), "requestId");
        Object capability = m.get("capability");
        String capStr = capability == null ? null : capability.toString();
        Object params = m.get("params");
        String paramsJson = params == null ? "{}" : Json.stringify(params);
        return new HostRequest(requestId, operationId, capStr, paramsJson);
    }

    private static long asLong(Object v, String field) {
        if (v instanceof Number) {
            return ((Number) v).longValue();
        }
        throw new IllegalArgumentException(field + " must be an integer, got: " + v);
    }
}
