package com.reader.core.host;

import java.util.Collections;
import java.util.Map;

/** Value object for an {@code http.execute} request descriptor (Core → Host). */
public final class HttpRequest {

    private final String url;
    private final String method;
    private final Map<String, String> headers;
    private final String body;

    public HttpRequest(String url, String method, Map<String, String> headers, String body) {
        this.url = url;
        this.method = method == null ? "GET" : method;
        this.headers = headers == null ? Collections.emptyMap() : headers;
        this.body = body;
    }

    public String url() {
        return url;
    }

    public String method() {
        return method;
    }

    public Map<String, String> headers() {
        return headers;
    }

    public String body() {
        return body;
    }
}
