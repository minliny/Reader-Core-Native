package com.reader.core.host;

import org.junit.jupiter.api.Test;

import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Deque;
import java.util.List;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicLong;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

/**
 * Integration tests for {@link HostRuntime}: the unified, concurrency-safe
 * facade that demultiplexes a single event stream into host.request handling
 * (adapter) and host-command correlation (sendAndAwait).
 */
class HostRuntimeTest {

    @Test
    void sendAndAwaitReceivesResultRoutedByPollThread() throws Exception {
        // On send, the transport enqueues a result event echoing the command's
        // requestId back, simulating Core's reply.
        ScriptedTransport transport = new ScriptedTransport();
        transport.onSend = cmd -> {
            long id = requestIdOf(cmd);
            transport.enqueueAfterDelay("{\"protocolVersion\":1,\"requestId\":" + id
                    + ",\"type\":\"result\",\"data\":{\"pong\":true}}");
        };

        HostRuntime rt = HostRuntime.over(transport, 50L).start();
        try {
            CommandResult res = rt.sendAndAwait("runtime.ping", "{}", 2000L);
            assertTrue(res.isSuccess());
            assertEquals("{\"pong\":true}", res.dataJson());
        } finally {
            rt.stop();
        }
        assertFalse(rt.isRunning());
    }

    @Test
    void sendAndAwaitReceivesErrorEvent() throws Exception {
        ScriptedTransport transport = new ScriptedTransport();
        transport.onSend = cmd -> {
            long id = requestIdOf(cmd);
            transport.enqueueAfterDelay("{\"protocolVersion\":1,\"requestId\":" + id
                    + ",\"type\":\"error\",\"error\":{\"code\":\"UNKNOWN_METHOD\","
                    + "\"message\":\"x\",\"retryable\":false}}");
        };

        HostRuntime rt = HostRuntime.over(transport, 50L).start();
        try {
            CommandResult res = rt.sendAndAwait("bogus", "{}", 2000L);
            assertTrue(res.isError());
            assertTrue(res.errorJson().contains("UNKNOWN_METHOD"));
        } finally {
            rt.stop();
        }
    }

    @Test
    void sendAndAwaitTimesOutWhenNoReply() throws Exception {
        ScriptedTransport transport = new ScriptedTransport(); // never enqueues
        HostRuntime rt = HostRuntime.over(transport, 50L).start();
        try {
            CommandResult res = rt.sendAndAwait("runtime.ping", "{}", 300L);
            assertTrue(res.isTimeout());
            assertEquals(0, rt.pendingCount());
        } finally {
            rt.stop();
        }
    }

    @Test
    void hostRequestEventsAreDispatchedToAdapterWhileAwaiting() throws Exception {
        // While the runtime waits for a command result, a host.request arrives
        // and must be answered — proving the single poll thread serves both.
        ScriptedTransport transport = new ScriptedTransport();
        final AtomicLong echoOperationId = new AtomicLong(-1);
        transport.onSend = cmd -> {
            // First send is the host.request reply (host.complete) from the
            // smoke handler; second is our runtime.ping command, which we answer.
            if (cmd.contains("\"method\":\"host.complete\"")) {
                // captured reply; nothing to enqueue
                return;
            }
            long id = requestIdOf(cmd);
            transport.enqueueAfterDelay("{\"protocolVersion\":1,\"requestId\":" + id
                    + ",\"type\":\"result\",\"data\":{\"ok\":1}}");
        };

        HostRuntime rt = HostRuntime.over(transport, 50L)
                .register(HostSmokeEchoHandler.CAPABILITY, new HostSmokeEchoHandler())
                .start();
        try {
            // Enqueue a host.request for the smoke capability — the runtime's
            // poll thread must dispatch it and send host.complete.
            transport.enqueue("{\"protocolVersion\":1,\"requestId\":5000,\"type\":\"host.request\","
                    + "\"operationId\":77,\"capability\":\"host.smoke.echo\","
                    + "\"params\":{\"m\":1}}");

            // Also send a command; both flow through the same poll thread.
            CommandResult res = rt.sendAndAwait("runtime.ping", "{}", 2000L);
            assertTrue(res.isSuccess());

            // The runtime must have emitted a host.complete reply for op 77.
            boolean sawComplete = false;
            for (String sent : transport.sent) {
                if (sent.contains("\"method\":\"host.complete\"")
                        && sent.contains("\"operationId\":77")) {
                    sawComplete = true;
                    break;
                }
            }
            assertTrue(sawComplete, "runtime should have replied to host.request");
        } finally {
            rt.stop();
        }
    }

    @Test
    void startStopIsIdempotent() {
        ScriptedTransport transport = new ScriptedTransport();
        HostRuntime rt = HostRuntime.over(transport);
        rt.start();
        rt.start();
        assertTrue(rt.isRunning());
        rt.stop();
        rt.stop();
        assertFalse(rt.isRunning());
    }

    @Test
    void stopFailsPendingSendAndAwaitPromptly() throws Exception {
        // No reply is ever enqueued, so sendAndAwait would block until timeout.
        // Calling stop() from another thread must fail the pending future
        // promptly with an error, well before the 5s timeout.
        ScriptedTransport transport = new ScriptedTransport();
        HostRuntime rt = HostRuntime.over(transport, 50L).start();

        long t0 = System.currentTimeMillis();
        Thread stopper = new Thread(() -> {
            try { Thread.sleep(150); } catch (InterruptedException e) {
                Thread.currentThread().interrupt(); return;
            }
            rt.stop();
        });
        stopper.start();

        CommandResult res = rt.sendAndAwait("runtime.ping", "{}", 5000L);
        long elapsed = System.currentTimeMillis() - t0;

        assertTrue(res.isError(), "pending sendAndAwait should be failed on stop");
        assertTrue(res.errorJson().contains("runtime stopped"));
        assertTrue(elapsed < 2000, "should return promptly after stop, took " + elapsed + "ms");
        stopper.join(2000);
    }

    @SuppressWarnings("unchecked")
    private static long requestIdOf(String commandJson) {
        Object root = Json.parse(commandJson);
        return ((Number) ((java.util.Map<String, Object>) root).get("requestId")).longValue();
    }

    /** Fake transport with a delayed-enqueue hook to mimic Core's async reply. */
    private static final class ScriptedTransport implements HostTransport {
        final Deque<String> events = new ArrayDeque<>();
        final List<String> sent = new ArrayList<>();
        volatile java.util.function.Consumer<String> onSend = cmd -> {};

        synchronized void enqueue(String e) {
            events.addLast(e);
        }

        /** Enqueue after yielding, so the sender's sendAndAwait is already waiting. */
        void enqueueAfterDelay(String e) {
            new Thread(() -> {
                try { Thread.sleep(10); } catch (InterruptedException ie) {
                    Thread.currentThread().interrupt(); return;
                }
                synchronized (this) {
                    events.addLast(e);
                    notifyAll();
                }
            }).start();
        }

        @Override
        public synchronized String pollEventJson(long timeoutMillis) {
            long deadline = System.currentTimeMillis() + Math.max(timeoutMillis, 0);
            while (events.isEmpty()) {
                long remaining = deadline - System.currentTimeMillis();
                if (remaining <= 0) return null;
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
            onSend.accept(commandJson);
        }
    }
}
