package com.reader.core.host;

import org.junit.jupiter.api.Test;

import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Deque;
import java.util.List;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

/**
 * End-to-end contract tests for {@link HostEventLoop} using a fake
 * {@link HostTransport}. Proves the full access path —
 * poll → parse → dispatch → encode → send — without the NDK library, and
 * asserts the outbound command matches the protocol conformance fixtures
 * (modulo the host-chosen outbound {@code requestId}).
 */
class HostEventLoopTest {

    @Test
    void tickDispatchesHostRequestAndSendsCompleteMatchingFixture() {
        FakeTransport transport = new FakeTransport();
        transport.enqueue(hostRequestEvent(1L, "host.smoke.echo",
                "{\"message\":\"conformance host request\"}"));

        HostAdapter adapter = new HostAdapter();
        adapter.register("host.smoke.echo", req ->
                HostReply.complete("{\"status\":\"ok\",\"source\":\"conformance\"}"));

        HostEventLoop loop = new HostEventLoop(transport, adapter);
        assertTrue(loop.tick());

        assertEquals(1, transport.sent.size());
        // First outbound requestId is 1000; body must match complete.json modulo requestId.
        String expected = withRequestId(fixture("complete.json"), 1000L);
        assertEquals(Json.canonicalize(expected), Json.canonicalize(transport.sent.get(0)));
        assertEquals(1, loop.repliedCount());
        assertEquals(0, loop.skippedCount());
    }

    @Test
    void tickDispatchesHostRequestAndSendsErrorMatchingFixture() {
        FakeTransport transport = new FakeTransport();
        transport.enqueue(hostRequestEvent(1L, "host.smoke.fail", "{}"));

        HostAdapter adapter = new HostAdapter();
        adapter.register("host.smoke.fail", req ->
                HostReply.error("INTERNAL", "host conformance failure", true));

        HostEventLoop loop = new HostEventLoop(transport, adapter);
        assertTrue(loop.tick());

        String expected = withRequestId(fixture("error.json"), 1000L);
        assertEquals(Json.canonicalize(expected), Json.canonicalize(transport.sent.get(0)));
    }

    @Test
    void tickIgnoresResultEvent() {
        FakeTransport transport = new FakeTransport();
        transport.enqueue("{\"protocolVersion\":1,\"requestId\":42,\"type\":\"result\",\"data\":{}}");

        HostEventLoop loop = new HostEventLoop(transport, new HostAdapter());
        assertFalse(loop.tick());
        assertEquals(0, transport.sent.size());
        assertEquals(1, loop.skippedCount());
        assertEquals(0, loop.repliedCount());
    }

    @Test
    void tickIgnoresErrorEvent() {
        FakeTransport transport = new FakeTransport();
        transport.enqueue("{\"protocolVersion\":1,\"requestId\":42,\"type\":\"error\","
                + "\"error\":{\"code\":\"INTERNAL\",\"message\":\"x\",\"retryable\":false}}");

        HostEventLoop loop = new HostEventLoop(transport, new HostAdapter());
        assertFalse(loop.tick());
        assertEquals(0, transport.sent.size());
    }

    @Test
    void tickSkipsMalformedHostRequestWithoutReplying() {
        FakeTransport transport = new FakeTransport();
        // type says host.request but body is malformed → no reliable operationId.
        transport.enqueue("{\"protocolVersion\":1,\"type\":\"host.request\",\"operationId\":0,"
                + "\"capability\":\"host.smoke.echo\",\"params\":{}}");

        HostEventLoop loop = new HostEventLoop(transport, new HostAdapter());
        assertFalse(loop.tick());
        assertEquals(0, transport.sent.size());
        assertEquals(1, loop.skippedCount());
    }

    @Test
    void tickReturnsFalseOnTimeout() {
        FakeTransport transport = new FakeTransport(); // empty queue → null
        HostEventLoop loop = new HostEventLoop(transport, new HostAdapter());
        assertFalse(loop.tick());
        assertEquals(0, loop.processedCount());
    }

    @Test
    void drainProcessesAllHostRequestsUntilNonHostEvent() {
        FakeTransport transport = new FakeTransport();
        transport.enqueue(hostRequestEvent(1L, "host.smoke.echo", "{}"));
        transport.enqueue(hostRequestEvent(2L, "host.smoke.echo", "{}"));
        transport.enqueue("{\"protocolVersion\":1,\"requestId\":9,\"type\":\"result\",\"data\":{}}");
        transport.enqueue(hostRequestEvent(3L, "host.smoke.echo", "{}"));

        HostAdapter adapter = new HostAdapter();
        adapter.register("host.smoke.echo", req -> HostReply.complete("{}"));

        HostEventLoop loop = new HostEventLoop(transport, adapter);
        loop.drain();

        // Two host.requests before the result event, then drain stops at the result.
        assertEquals(2, transport.sent.size());
        assertEquals(2, loop.repliedCount());
        assertEquals(1, loop.skippedCount());
        // Outbound requestIds increment: 1000, 1001.
        long firstId = requestIdOf(transport.sent.get(0));
        long secondId = requestIdOf(transport.sent.get(1));
        assertEquals(1000L, firstId);
        assertEquals(1001L, secondId);
    }

    @Test
    void unsupportedCapabilitySendsInternalError() {
        FakeTransport transport = new FakeTransport();
        transport.enqueue(hostRequestEvent(1L, "host.unregistered", "{}"));

        HostEventLoop loop = new HostEventLoop(transport, new HostAdapter());
        assertTrue(loop.tick());

        assertEquals(1, transport.sent.size());
        String sent = transport.sent.get(0);
        assertTrue(sent.contains("\"method\":\"host.error\""));
        assertTrue(sent.contains("\"code\":\"INTERNAL\""));
        assertTrue(sent.contains("\"retryable\":false"));
    }

    // --- helpers ----------------------------------------------------------------

    private static String hostRequestEvent(long operationId, String capability, String paramsJson) {
        return "{\"protocolVersion\":1,\"requestId\":301,\"type\":\"host.request\","
                + "\"operationId\":" + operationId
                + ",\"capability\":\"" + capability + "\""
                + ",\"params\":" + paramsJson + "}";
    }

    @SuppressWarnings("unchecked")
    private static String withRequestId(String fixtureJson, long requestId) {
        Object root = Json.parse(fixtureJson);
        ((java.util.Map<String, Object>) root).put("requestId", requestId);
        return Json.stringify(root);
    }

    @SuppressWarnings("unchecked")
    private static long requestIdOf(String commandJson) {
        Object root = Json.parse(commandJson);
        return ((Number) ((java.util.Map<String, Object>) root).get("requestId")).longValue();
    }

    private static String fixture(String name) {
        try (java.io.InputStream in = HostEventLoopTest.class
                .getResourceAsStream("/conformance/host/" + name)) {
            if (in == null) throw new AssertionError("missing fixture: " + name);
            return new String(in.readAllBytes(), java.nio.charset.StandardCharsets.UTF_8);
        } catch (java.io.IOException e) {
            throw new AssertionError("failed reading fixture " + name, e);
        }
    }

    private static final class FakeTransport implements HostTransport {
        final Deque<String> events = new ArrayDeque<>();
        final List<String> sent = new ArrayList<>();

        void enqueue(String eventJson) {
            events.addLast(eventJson);
        }

        @Override
        public String pollEventJson(long timeoutMillis) {
            return events.pollFirst(); // null when empty → simulates timeout
        }

        @Override
        public void sendCommand(String commandJson) {
            sent.add(commandJson);
        }
    }
}
