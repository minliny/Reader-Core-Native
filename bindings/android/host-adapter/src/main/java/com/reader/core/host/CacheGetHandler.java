package com.reader.core.host;

import java.util.LinkedHashMap;
import java.util.Map;

/**
 * {@link CapabilityHandler} for the {@code cache.get} capability.
 *
 * <p>Progressive delivery — reports a cache miss by default. Real backing
 * store wiring (in-memory LRU, disk cache, etc.) is host-app work; this stub
 * keeps the Core pipeline unblocked without a persistence backend in pure-JVM
 * code.
 *
 * <p>Request params: {@code {key}} (string). Result: {@code {value: null, hit: false}}.
 */
public final class CacheGetHandler implements CapabilityHandler {

    public static final String CAPABILITY = "cache.get";

    private static final String INTERNAL = "INTERNAL";

    public CacheGetHandler() {
    }

    @Override
    @SuppressWarnings("unchecked")
    public HostReply handle(HostRequest request) {
        Object parsed;
        try {
            parsed = Json.parse(request.paramsJson());
        } catch (Json.JsonException e) {
            return HostReply.error(INTERNAL,
                    "invalid cache.get params: " + e.getMessage(), false);
        }
        if (!(parsed instanceof Map)) {
            return HostReply.error(INTERNAL, "cache.get params must be an object", false);
        }
        Object keyVal = ((Map<String, Object>) parsed).get("key");
        if (!(keyVal instanceof String) || ((String) keyVal).isEmpty()) {
            return HostReply.error(INTERNAL, "cache.get requires non-empty key", false);
        }
        Map<String, Object> result = new LinkedHashMap<>();
        result.put("value", null);
        result.put("hit", false);
        return HostReply.complete(Json.stringify(result));
    }
}
