package com.reader.core.host;

import org.junit.jupiter.api.Test;

import java.util.ArrayDeque;
import java.util.Deque;
import java.util.List;
import java.util.ArrayList;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

/**
 * Contract tests for {@link HostCommander} — the host-initiated command/response
 * half of the protocol. Sends a command via the transport and correlates the
 * matching {@code result} / {@code error} event by {@code requestId}.
 */
class HostCommanderTest {

    @Test
    void sendAndAwaitMatchesResultEventByRequestId() {
        FakeTransport transport = new FakeTransport();
        // The commander sends requestId 2000; Core replies with a result event.
        transport.onSend = cmd -> transport.enqueue(
                "{\"protocolVersion\":1,\"requestId\":2000,\"type\":\"result\","
                        + "\"data\":{\"pong\":true}}");

        HostCommander commander = new HostCommander(transport);
        CommandResult res = commander.sendAndAwait("runtime.ping", "{}", 1000L);

        assertTrue(res.isSuccess());
        assertEquals(2000L, res.requestId());
        assertEquals("{\"pong\":true}", res.dataJson());
        // The sent command must carry protocolVersion + method + params.
        assertEquals("{\"protocolVersion\":1,\"requestId\":2000,"
                + "\"method\":\"runtime.ping\",\"params\":{}}",
                transport.sent.get(0));
    }

    @Test
    void sendAndAwaitMatchesErrorEventByRequestId() {
        FakeTransport transport = new FakeTransport();
        transport.onSend = cmd -> transport.enqueue(
                "{\"protocolVersion\":1,\"requestId\":2000,\"type\":\"error\","
                        + "\"error\":{\"code\":\"UNKNOWN_METHOD\",\"message\":\"x\",\"retryable\":false}}");

        HostCommander commander = new HostCommander(transport);
        CommandResult res = commander.sendAndAwait("bogus", "{}", 1000L);

        assertTrue(res.isError());
        assertFalse(res.isTimeout());
        assertTrue(res.errorJson().contains("\"UNKNOWN_METHOD\""));
    }

    @Test
    void sendAndAwaitReturnsTimeoutWhenNoMatchingEvent() {
        FakeTransport transport = new FakeTransport();
        // No event ever enqueued → poll returns null → commander times out.
        HostCommander commander = new HostCommander(transport);
        CommandResult res = commander.sendAndAwait("runtime.ping", "{}", 200L);

        assertTrue(res.isTimeout());
        assertEquals(2000L, res.requestId());
    }

    @Test
    void ignoresEventsForOtherRequestIds() {
        FakeTransport transport = new FakeTransport();
        transport.onSend = cmd -> {
            // A stray event for a different requestId arrives first, then ours.
            transport.enqueue("{\"protocolVersion\":1,\"requestId\":9999,\"type\":\"result\",\"data\":{}}");
            transport.enqueue("{\"protocolVersion\":1,\"requestId\":2000,\"type\":\"result\",\"data\":{\"ok\":1}}");
        };

        HostCommander commander = new HostCommander(transport);
        CommandResult res = commander.sendAndAwait("runtime.status", "{}", 1000L);

        assertTrue(res.isSuccess());
        assertEquals("{\"ok\":1}", res.dataJson());
    }

    @Test
    void requestIdIncrementsAcrossCalls() {
        FakeTransport transport = new FakeTransport();
        transport.onSend = cmd -> {
            long id = requestIdOf(cmd);
            transport.enqueue("{\"protocolVersion\":1,\"requestId\":" + id
                    + ",\"type\":\"result\",\"data\":{}}");
        };

        HostCommander commander = new HostCommander(transport);
        commander.sendAndAwait("a", "{}", 1000L);
        commander.sendAndAwait("b", "{}", 1000L);

        assertEquals(2002L, commander.nextRequestId());
        assertEquals(2000L, requestIdOf(transport.sent.get(0)));
        assertEquals(2001L, requestIdOf(transport.sent.get(1)));
    }

    @Test
    void encodeCommandUsesJsonStringifyForMethod() {
        // Method string with a quote must be JSON-escaped, not interpolated raw.
        String cmd = HostCommander.encodeCommand(5L, "runtime.\"ping\"", "{}");
        assertEquals("{\"protocolVersion\":1,\"requestId\":5,"
                + "\"method\":\"runtime.\\\"ping\\\"\",\"params\":{}}", cmd);
    }

    @SuppressWarnings("unchecked")
    private static long requestIdOf(String commandJson) {
        Object root = Json.parse(commandJson);
        return ((Number) ((java.util.Map<String, Object>) root).get("requestId")).longValue();
    }

    private static final class FakeTransport implements HostTransport {
        final Deque<String> events = new ArrayDeque<>();
        final List<String> sent = new ArrayList<>();
        java.util.function.Consumer<String> onSend = cmd -> {};

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
            onSend.accept(commandJson);
        }
    }
}
