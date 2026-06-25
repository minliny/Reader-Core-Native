package com.reader.core.host;

import org.junit.jupiter.api.Test;

import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Deque;
import java.util.List;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

/**
 * Integration tests for {@link HostBus}: the host-side product surface over the
 * Core ABI/protocol access path. Covers synchronous tick/drain scripting and an
 * asynchronous daemon-thread run driven by a blocking fake transport.
 */
class HostBusTest {

    @Test
    void tickIgnoresResultEventAndRepliesToHostRequest() {
        FakeTransport transport = new FakeTransport();
        transport.enqueue("{\"protocolVersion\":1,\"requestId\":42,\"type\":\"result\",\"data\":{}}");
        transport.enqueue(hostRequest(1L, "host.smoke.echo", "{\"message\":\"hi\"}"));

        HostBus bus = HostBus.over(transport)
                .register("host.smoke.echo", new HostSmokeEchoHandler());

        // First tick: result event is ignored.
        assertFalse(bus.tick());
        assertEquals(0, bus.repliedCount());

        // Second tick: host.request is echoed back as host.complete.
        assertTrue(bus.tick());
        assertEquals(1, bus.repliedCount());
        assertEquals(1, transport.sent.size());
        String sent = transport.sent.get(0);
        assertTrue(sent.contains("\"method\":\"host.complete\""));
        assertTrue(sent.contains("\"message\":\"hi\""));
    }

    @Test
    void drainProcessesAllPendingHostRequests() {
        FakeTransport transport = new FakeTransport();
        transport.enqueue(hostRequest(1L, "http.execute", "{\"url\":\"https://a.test\"}"));
        transport.enqueue(hostRequest(2L, "http.execute", "{\"url\":\"https://b.test\"}"));
        transport.enqueue("{\"protocolVersion\":1,\"requestId\":9,\"type\":\"result\",\"data\":{}}");

        HttpFetch fetch = req -> new HttpResponse(200, "body:" + req.url());
        HostBus bus = HostBus.over(transport)
                .register("http.execute", new HttpExecuteHandler(fetch));

        bus.drain();
        assertEquals(2, bus.repliedCount());
        assertEquals(1, bus.skippedCount());
        assertTrue(bus.transport() == transport);
    }

    @Test
    void unsupportedCapabilityYieldsInternalErrorViaBus() {
        FakeTransport transport = new FakeTransport();
        transport.enqueue(hostRequest(1L, "host.unregistered", "{}"));

        HostBus bus = HostBus.over(transport);
        assertTrue(bus.tick());
        assertEquals(1, transport.sent.size());
        assertTrue(transport.sent.get(0).contains("\"method\":\"host.error\""));
        assertTrue(transport.sent.get(0).contains("\"code\":\"INTERNAL\""));
    }

    @Test
    void startStopDrivesAsyncLoopWithBlockingTransport() throws InterruptedException {
        BlockingTransport transport = new BlockingTransport();
        transport.enqueue(hostRequest(1L, "host.smoke.echo", "{\"m\":1}"));
        transport.enqueue(hostRequest(2L, "host.smoke.echo", "{\"m\":2}"));
        // No more events: the third poll blocks until its 50ms timeout, returns
        // null, and the loop spins quietly until stop().

        CountDownLatch twoReplies = new CountDownLatch(2);
        HostBus bus = HostBus.over(transport, 50L)
                .register("host.smoke.echo", req -> {
                    HostReply r = new HostSmokeEchoHandler().handle(req);
                    twoReplies.countDown();
                    return r;
                });

        bus.start();
        assertTrue(bus.isRunning());
        try {
            assertTrue(twoReplies.await(2, TimeUnit.SECONDS),
                    "expected two host.request replies within timeout");
            assertEquals(2, bus.repliedCount());
        } finally {
            bus.stop();
        }
        assertFalse(bus.isRunning());
    }

    @Test
    void stopIsIdempotentAndStartIsIdempotent() {
        FakeTransport transport = new FakeTransport();
        HostBus bus = HostBus.over(transport);
        bus.start();
        bus.start(); // no-op
        assertTrue(bus.isRunning());
        bus.stop();
        bus.stop(); // no-op
        assertFalse(bus.isRunning());
    }

    // --- helpers ----------------------------------------------------------------

    private static String hostRequest(long operationId, String capability, String paramsJson) {
        return "{\"protocolVersion\":1,\"requestId\":301,\"type\":\"host.request\","
                + "\"operationId\":" + operationId
                + ",\"capability\":\"" + capability + "\""
                + ",\"params\":" + paramsJson + "}";
    }

    private static final class FakeTransport implements HostTransport {
        final Deque<String> events = new ArrayDeque<>();
        final List<String> sent = new ArrayList<>();

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

    /** Blocks on poll until an event is enqueued, simulating the real callback queue. */
    private static final class BlockingTransport implements HostTransport {
        private final Deque<String> events = new ArrayDeque<>();
        final List<String> sent = new ArrayList<>();

        synchronized void enqueue(String e) {
            events.addLast(e);
            notifyAll();
        }

        @Override
        public synchronized String pollEventJson(long timeoutMillis) {
            long deadline = System.currentTimeMillis() + Math.max(timeoutMillis, 0);
            while (events.isEmpty()) {
                long remaining = deadline - System.currentTimeMillis();
                if (remaining <= 0) {
                    return null;
                }
                try {
                    wait(remaining);
                } catch (InterruptedException e) {
                    Thread.currentThread().interrupt();
                    return null;
                }
            }
            return events.pollFirst();
        }

        @Override
        public synchronized void sendCommand(String commandJson) {
            sent.add(commandJson);
        }
    }
}
