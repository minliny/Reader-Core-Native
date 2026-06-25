package com.reader.core.host;

import java.util.LinkedHashMap;
import java.util.Map;

/**
 * {@link CapabilityHandler} for the {@code http.execute} shared-contract
 * capability (see {@code protocol/compatpatibility.md} §"HTTP transport
 * capability" and {@code docs/host-app-contracts/01-network-session.md}).
 *
 * <p>Core sends a {@code host.request} with params
 * {@code {url, method, headers, body}}; this handler delegates the actual
 * network fetch to the host-owned {@link HttpFetch} mechanism and returns a
 * {@code host.complete} whose result is {@code {status, body}} (or
 * {@code {status, headers, body}} when the fetch supplies headers). It never
 * opens a socket itself — TLS / network policy / cookies stay on the host side.
 *
 * <p>Failure mapping (transport-level error codes are a known protocol gap, so
 * host errors use the fixture-backed {@code INTERNAL} code):
 * <ul>
 *   <li>missing/invalid {@code url} → non-retryable INTERNAL</li>
 *   <li>{@link HttpFetch} throws → retryable INTERNAL (transient host failure)</li>
 * </ul>
 */
public final class HttpExecuteHandler implements CapabilityHandler {

    public static final String CAPABILITY = "http.execute";

    private static final String INTERNAL = "INTERNAL";

    private final HttpFetch fetch;

    public HttpExecuteHandler(HttpFetch fetch) {
        if (fetch == null) {
            throw new IllegalArgumentException("fetch required");
        }
        this.fetch = fetch;
    }

    @Override
    @SuppressWarnings("unchecked")
    public HostReply handle(HostRequest request) {
        Object parsed;
        try {
            parsed = Json.parse(request.paramsJson());
        } catch (Json.JsonException e) {
            return HostReply.error(INTERNAL, "invalid http.execute params: " + e.getMessage(), false);
        }
        if (!(parsed instanceof Map)) {
            return HostReply.error(INTERNAL, "http.execute params must be an object", false);
        }
        Map<String, Object> params = (Map<String, Object>) parsed;

        Object urlVal = params.get("url");
        if (!(urlVal instanceof String) || ((String) urlVal).isEmpty()) {
            return HostReply.error(INTERNAL, "http.execute requires non-empty url", false);
        }
        String url = (String) urlVal;
        String method = stringOrDefault(params.get("method"), "GET");
        Map<String, String> headers = toStringMap(params.get("headers"));
        String body = params.get("body") == null ? null : params.get("body").toString();

        HttpRequest httpReq = new HttpRequest(url, method, headers, body);
        HttpResponse httpRes;
        try {
            httpRes = fetch.fetch(httpReq);
        } catch (Exception e) {
            return HostReply.error(INTERNAL,
                    "http.execute fetch failed: " + e.getMessage(), true);
        }
        return HostReply.complete(buildResultJson(httpRes));
    }

    private static String buildResultJson(HttpResponse res) {
        Map<String, Object> result = new LinkedHashMap<>();
        result.put("status", res.status());
        if (res.hasHeaders()) {
            result.put("headers", res.headers());
        }
        result.put("body", res.body());
        return Json.stringify(result);
    }

    private static String stringOrDefault(Object v, String def) {
        if (v == null) return def;
        String s = v.toString();
        return s.isEmpty() ? def : s;
    }

    @SuppressWarnings("unchecked")
    private static Map<String, String> toStringMap(Object v) {
        if (!(v instanceof Map)) {
            return java.util.Collections.emptyMap();
        }
        Map<String, String> out = new LinkedHashMap<>();
        for (Map.Entry<String, Object> e : ((Map<String, Object>) v).entrySet()) {
            out.put(e.getKey(), e.getValue() == null ? null : e.getValue().toString());
        }
        return out;
    }
}
