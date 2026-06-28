package com.reader.core.host;

import java.util.LinkedHashMap;
import java.util.Map;

/**
 * {@link CapabilityHandler} for the {@code persistence.get} capability.
 *
 * <p>Progressive delivery — reports a miss (null value) by default. Real
 * persistence backend wiring (SharedPreferences, DataStore, SQLite, etc.) is
 * host-app work; this stub keeps the Core pipeline unblocked without a
 * persistence layer in pure-JVM code.
 *
 * <p>Request params: {@code {key}} (string). Result: {@code {value: null}}.
 */
public final class PersistenceGetHandler implements CapabilityHandler {

    public static final String CAPABILITY = "persistence.get";

    private static final String INTERNAL = "INTERNAL";

    public PersistenceGetHandler() {
    }

    @Override
    @SuppressWarnings("unchecked")
    public HostReply handle(HostRequest request) {
        Object parsed;
        try {
            parsed = Json.parse(request.paramsJson());
        } catch (Json.JsonException e) {
            return HostReply.error(INTERNAL,
                    "invalid persistence.get params: " + e.getMessage(), false);
        }
        if (!(parsed instanceof Map)) {
            return HostReply.error(INTERNAL, "persistence.get params must be an object", false);
        }
        Object keyVal = ((Map<String, Object>) parsed).get("key");
        if (!(keyVal instanceof String) || ((String) keyVal).isEmpty()) {
            return HostReply.error(INTERNAL, "persistence.get requires non-empty key", false);
        }
        Map<String, Object> result = new LinkedHashMap<>();
        result.put("value", null);
        return HostReply.complete(Json.stringify(result));
    }
}
