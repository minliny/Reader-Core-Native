package com.reader.core.host;

/**
 * Host-owned HTTP fetch mechanism. The adapter calls this to actually perform
 * the network operation for an {@code http.execute} host request; the platform
 * supplies the real implementation (OkHttp / {@code HttpURLConnection} /
 * Cronet). Keeping it behind an interface lets {@link HttpExecuteHandler} be
 * unit-tested with a fake, and keeps TLS / socket / network policy firmly on
 * the host side — the adapter never touches the network directly.
 *
 * <p>Request/result shapes follow {@code protocol/compatibility.md} §"HTTP
 * transport capability" and {@code docs/host-app-contracts/01-network-session.md}:
 * <pre>
 *   request:  { "url", "method", "headers", "body" }
 *   response: { "status", "body" }  (+ optional "headers")
 * </pre>
 */
public interface HttpFetch {

    HttpResponse fetch(HttpRequest request) throws Exception;
}
