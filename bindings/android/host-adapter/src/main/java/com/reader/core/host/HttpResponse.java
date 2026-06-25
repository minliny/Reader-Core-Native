package com.reader.core.host;

import java.util.Map;

/** Value object for an {@code http.execute} response (Host → Core). */
public final class HttpResponse {

    private final int status;
    private final String body;
    private final Map<String, String> headers;

    public HttpResponse(int status, String body) {
        this(status, body, null);
    }

    public HttpResponse(int status, String body, Map<String, String> headers) {
        this.status = status;
        this.body = body == null ? "" : body;
        this.headers = headers == null ? null : headers;
    }

    public int status() {
        return status;
    }

    public String body() {
        return body;
    }

    public Map<String, String> headers() {
        return headers;
    }

    public boolean hasHeaders() {
        return headers != null;
    }
}
