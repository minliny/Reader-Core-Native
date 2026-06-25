package com.reader.core.host;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.util.LinkedHashMap;
import java.util.Map;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

/**
 * Contract tests for {@link HttpExecuteHandler}. The handler is driven through
 * {@link HostAdapter} / {@link HostEventLoop} with a fake {@link HttpFetch};
 * the encoded {@code host.complete} command is checked against
 * {@code protocol/fixtures/conformance/host/http-complete-with-metadata.json}.
 */
class HttpExecuteHandlerTest {

    @Test
    void completeResultShapeMatchesBaseContract() {
        FakeFetch fetch = new FakeFetch(new HttpResponse(200, "hello"));
        HostAdapter adapter = new HostAdapter();
        adapter.register(HttpExecuteHandler.CAPABILITY, new HttpExecuteHandler(fetch));

        HostReply reply = adapter.dispatch(request("{\"url\":\"https://x.test\",\"method\":\"GET\"}"));
        assertTrue(reply.isComplete());
        assertEquals("{\"status\":200,\"body\":\"hello\"}",
                ((HostReply.Complete) reply).resultJson());
        assertEquals("https://x.test", fetch.lastRequest.url());
        assertEquals("GET", fetch.lastRequest.method());
    }

    @Test
    void defaultsMethodToGet() {
        FakeFetch fetch = new FakeFetch(new HttpResponse(204, ""));
        HostAdapter adapter = new HostAdapter();
        adapter.register(HttpExecuteHandler.CAPABILITY, new HttpExecuteHandler(fetch));

        adapter.dispatch(request("{\"url\":\"https://x.test\"}"));
        assertEquals("GET", fetch.lastRequest.method());
    }

    @Test
    void completeWithHeadersMatchesFixtureViaEventLoop() {
        FakeFetch fetch = new FakeFetch(new HttpResponse(200,
                "{\"books\":[]}",
                Map.of("content-type", "application/json")));

        HostAdapter adapter = new HostAdapter();
        adapter.register(HttpExecuteHandler.CAPABILITY, new HttpExecuteHandler(fetch));

        FakeTransport transport = new FakeTransport();
        transport.enqueue("{\"protocolVersion\":1,\"requestId\":301,\"type\":\"host.request\","
                + "\"operationId\":1,\"capability\":\"http.execute\","
                + "\"params\":{\"url\":\"https://example.test/path\",\"method\":\"GET\","
                + "\"headers\":{},\"body\":null}}");

        HostEventLoop loop = new HostEventLoop(transport, adapter);
        assertTrue(loop.tick());

        // Outbound requestId is the loop's first (1000); body must match the
        // http-complete-with-metadata fixture modulo requestId.
        String expected = withRequestId(fixture("http-complete-with-metadata.json"), 1000L);
        assertEquals(Json.canonicalize(expected), Json.canonicalize(transport.sent.get(0)));
    }

    @Test
    void missingUrlYieldsNonRetryableInternalError() {
        FakeFetch fetch = new FakeFetch(new HttpResponse(200, ""));
        HostAdapter adapter = new HostAdapter();
        adapter.register(HttpExecuteHandler.CAPABILITY, new HttpExecuteHandler(fetch));

        HostReply reply = adapter.dispatch(request("{\"method\":\"GET\"}"));
        assertTrue(reply.isError());
        HostReply.Error err = (HostReply.Error) reply;
        assertEquals("INTERNAL", err.code());
        assertFalse(err.retryable());
    }

    @Test
    void fetchThrowingYieldsRetryableInternalError() {
        FakeFetch fetch = new FakeFetch(new java.io.IOException("timeout"));
        HostAdapter adapter = new HostAdapter();
        adapter.register(HttpExecuteHandler.CAPABILITY, new HttpExecuteHandler(fetch));

        HostReply reply = adapter.dispatch(request("{\"url\":\"https://x.test\"}"));
        assertTrue(reply.isError());
        HostReply.Error err = (HostReply.Error) reply;
        assertEquals("INTERNAL", err.code());
        assertTrue(err.retryable());
    }

    @Test
    void forwardsHeadersAndBodyToFetch() {
        FakeFetch fetch = new FakeFetch(new HttpResponse(200, "ok"));
        HostAdapter adapter = new HostAdapter();
        adapter.register(HttpExecuteHandler.CAPABILITY, new HttpExecuteHandler(fetch));

        Map<String, Object> headers = new LinkedHashMap<>();
        headers.put("User-Agent", "reader");
        headers.put("Cookie", "sid=1");

        adapter.dispatch(request("{\"url\":\"https://x.test\",\"method\":\"POST\","
                + "\"headers\":{\"User-Agent\":\"reader\",\"Cookie\":\"sid=1\"},"
                + "\"body\":\"{\\\"q\\\":1}\"}"));
        assertEquals("POST", fetch.lastRequest.method());
        assertEquals("reader", fetch.lastRequest.headers().get("User-Agent"));
        assertEquals("sid=1", fetch.lastRequest.headers().get("Cookie"));
        assertEquals("{\"q\":1}", fetch.lastRequest.body());
    }

    // --- helpers ----------------------------------------------------------------

    private static HostRequest request(String paramsJson) {
        return HostRequest.parse("{\"protocolVersion\":1,\"requestId\":301,"
                + "\"type\":\"host.request\",\"operationId\":1,"
                + "\"capability\":\"http.execute\",\"params\":" + paramsJson + "}");
    }

    @SuppressWarnings("unchecked")
    private static String withRequestId(String fixtureJson, long requestId) {
        Object root = Json.parse(fixtureJson);
        ((Map<String, Object>) root).put("requestId", requestId);
        return Json.stringify(root);
    }

    private static String fixture(String name) {
        try (InputStream in = HttpExecuteHandlerTest.class
                .getResourceAsStream("/conformance/host/" + name)) {
            if (in == null) throw new AssertionError("missing fixture: " + name);
            return new String(in.readAllBytes(), StandardCharsets.UTF_8);
        } catch (IOException e) {
            throw new AssertionError("failed reading fixture " + name, e);
        }
    }

    private static final class FakeFetch implements HttpFetch {
        final HttpResponse next;
        final Exception failure;
        HttpRequest lastRequest;

        FakeFetch(HttpResponse next) {
            this.next = next;
            this.failure = null;
        }

        FakeFetch(Exception failure) {
            this.next = null;
            this.failure = failure;
        }

        @Override
        public HttpResponse fetch(HttpRequest request) throws Exception {
            this.lastRequest = request;
            if (failure != null) {
                throw failure;
            }
            return next;
        }
    }

    private static final class FakeTransport implements HostTransport {
        final java.util.Deque<String> events = new java.util.ArrayDeque<>();
        final java.util.List<String> sent = new java.util.ArrayList<>();

        void enqueue(String e) {
            events.addLast(e);
        }

        @Override
        public String pollEventJson(long timeoutMillis) {
            return events.pollFirst();
        }

        @Override
        public void sendCommand(String commandJson) {
            sent.add(commandJson);
        }
    }
}
