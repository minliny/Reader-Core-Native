package com.reader.core.host;

import java.util.LinkedHashMap;
import java.util.Map;

/**
 * {@link CapabilityHandler} for the {@code cookie.set} capability.
 *
 * <p>Progressive delivery — accepts the request and reports success without
 * touching platform cookie storage. Real CookieManager wiring (Android
 * {@code WebView.CookieManager}, etc.) is host-app work; this stub keeps the
 * Core pipeline unblocked from pure-JVM code.
 *
 * <p>Request params: {@code {url, cookies}}. Result: {@code {set: true}}.
 */
public final class CookieSetHandler implements CapabilityHandler {

    public static final String CAPABILITY = "cookie.set";

    private static final String INTERNAL = "INTERNAL";

    public CookieSetHandler() {
    }

    @Override
    @SuppressWarnings("unchecked")
    public HostReply handle(HostRequest request) {
        Object parsed;
        try {
            parsed = Json.parse(request.paramsJson());
        } catch (Json.JsonException e) {
            return HostReply.error(INTERNAL,
                    "invalid cookie.set params: " + e.getMessage(), false);
        }
        if (!(parsed instanceof Map)) {
            return HostReply.error(INTERNAL, "cookie.set params must be an object", false);
        }
        Object urlVal = ((Map<String, Object>) parsed).get("url");
        if (!(urlVal instanceof String) || ((String) urlVal).isEmpty()) {
            return HostReply.error(INTERNAL, "cookie.set requires non-empty url", false);
        }
        Map<String, Object> result = new LinkedHashMap<>();
        result.put("set", true);
        return HostReply.complete(Json.stringify(result));
    }
}
