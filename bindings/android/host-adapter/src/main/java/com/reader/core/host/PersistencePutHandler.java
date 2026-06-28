package com.reader.core.host;

import java.util.LinkedHashMap;
import java.util.Map;

/**
 * {@link CapabilityHandler} for the {@code persistence.put} capability.
 *
 * <p>Progressive delivery — accepts the entry and reports success without a
 * real backing store. Real persistence backend wiring (SharedPreferences,
 * DataStore, SQLite, etc.) is host-app work; this stub keeps the Core pipeline
 * unblocked from pure-JVM code.
 *
 * <p>Request params: {@code {key, value}}. Result: {@code {stored: true}}.
 */
public final class PersistencePutHandler implements CapabilityHandler {

    public static final String CAPABILITY = "persistence.put";

    private static final String INTERNAL = "INTERNAL";

    public PersistencePutHandler() {
    }

    @Override
    @SuppressWarnings("unchecked")
    public HostReply handle(HostRequest request) {
        Object parsed;
        try {
            parsed = Json.parse(request.paramsJson());
        } catch (Json.JsonException e) {
            return HostReply.error(INTERNAL,
                    "invalid persistence.put params: " + e.getMessage(), false);
        }
        if (!(parsed instanceof Map)) {
            return HostReply.error(INTERNAL, "persistence.put params must be an object", false);
        }
        Object keyVal = ((Map<String, Object>) parsed).get("key");
        if (!(keyVal instanceof String) || ((String) keyVal).isEmpty()) {
            return HostReply.error(INTERNAL, "persistence.put requires non-empty key", false);
        }
        Map<String, Object> result = new LinkedHashMap<>();
        result.put("stored", true);
        return HostReply.complete(Json.stringify(result));
    }
}
