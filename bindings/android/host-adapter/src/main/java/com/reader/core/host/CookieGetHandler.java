package com.reader.core.host;

import java.util.LinkedHashMap;
import java.util.Map;

/**
 * {@link CapabilityHandler} for the {@code cookie.get} capability.
 *
 * <p>Progressive delivery — returns an empty cookie map by default. Real
 * platform CookieManager wiring (Android {@code WebView.CookieManager}, etc.)
 * is host-app work; this stub keeps the Core pipeline unblocked without
 * touching platform cookie storage from pure-JVM code.
 *
 * <p>Request params: {@code {url}} (string). Result: {@code {cookies: {}}}.
 */
public final class CookieGetHandler implements CapabilityHandler {

    public static final String CAPABILITY = "cookie.get";

    private static final String INTERNAL = "INTERNAL";

    public CookieGetHandler() {
    }

    @Override
    @SuppressWarnings("unchecked")
    public HostReply handle(HostRequest request) {
        Object parsed;
        try {
            parsed = Json.parse(request.paramsJson());
        } catch (Json.JsonException e) {
            return HostReply.error(INTERNAL,
                    "invalid cookie.get params: " + e.getMessage(), false);
        }
        if (!(parsed instanceof Map)) {
            return HostReply.error(INTERNAL, "cookie.get params must be an object", false);
        }
        Object urlVal = ((Map<String, Object>) parsed).get("url");
        if (!(urlVal instanceof String) || ((String) urlVal).isEmpty()) {
            return HostReply.error(INTERNAL, "cookie.get requires non-empty url", false);
        }
        Map<String, Object> result = new LinkedHashMap<>();
        result.put("cookies", new LinkedHashMap<String, Object>());
        return HostReply.complete(Json.stringify(result));
    }
}
