package com.reader.core.host;

import org.junit.jupiter.api.Test;

import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Deque;
import java.util.List;
import java.util.concurrent.Executors;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

/**
 * Verifies the {@link HostRuntime#dispatchExecutor} offload: a slow capability
 * handler must not block the poll thread from correlating a concurrent
 * host-initiated command's result.
 */
class HostRuntimeExecutorTest {

    @Test
    void slowHandlerDoesNotBlockCommandResultWhenExecutorConfigured() throws Exception {
        ScriptedTransport transport = new ScriptedTransport();
        // Answer host-initiated commands immediately.
        transport.onSend = cmd -> {
            if (cmd.contains("\"method\":\"runtime.ping\"")) {
                long id = requestIdOf(cmd);
                transport.enqueueAfterDelay("{\"protocolVersion\":1,\"requestId\":" + id
                        + ",\"type\":\"result\",\"data\":{\"pong\":true}}");
            }
        };

        AtomicInteger slowCalls = new AtomicInteger();
        CapabilityHandler slowHandler = req -> {
            try {
                Thread.sleep(500); // simulate a slow HTTP fetch
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
            }
            slowCalls.incrementAndGet();
            return HostReply.complete("{\"ok\":true}");
        };

        ExecutorService pool = Executors.newSingleThreadExecutor();
        HostRuntime rt = HostRuntime.over(transport, 50L)
                .register("host.slow", slowHandler)
                .dispatchExecutor(pool)
                .start();
        try {
            // Enqueue a slow host.request, then immediately send a command.
            transport.enqueue("{\"protocolVersion\":1,\"requestId\":5001,\"type\":\"host.request\","
                    + "\"operationId\":99,\"capability\":\"host.slow\",\"params\":{}}");

            long t0 = System.currentTimeMillis();
            CommandResult res = rt.sendAndAwait("runtime.ping", "{}", 3000L);
            long elapsed = System.currentTimeMillis() - t0;

            assertTrue(res.isSuccess());
            // The command result should arrive well before the 500ms slow handler
            // finishes — proving dispatch was offloaded off the poll thread.
            assertTrue(elapsed < 500,
                    "sendAndAwait should not wait for the slow handler; took " + elapsed + "ms");
        } finally {
            rt.stop();
            pool.shutdown();
            assertTrue(pool.awaitTermination(5, TimeUnit.SECONDS));
        }
        assertEquals(1, slowCalls.get());
    }

    @Test
    void synchronousDispatchStillWorksWhenNoExecutor() throws Exception {
        // Sanity: without an executor, dispatch is on the poll thread (blocking).
        ScriptedTransport transport = new ScriptedTransport();
        transport.onSend = cmd -> {
            if (cmd.contains("\"method\":\"runtime.ping\"")) {
                long id = requestIdOf(cmd);
                transport.enqueueAfterDelay("{\"protocolVersion\":1,\"requestId\":" + id
                        + ",\"type\":\"result\",\"data\":{\"pong\":true}}");
            }
        };

        HostRuntime rt = HostRuntime.over(transport, 50L)
                .register(HostSmokeEchoHandler.CAPABILITY, new HostSmokeEchoHandler())
                .start();
        try {
            transport.enqueue("{\"protocolVersion\":1,\"requestId\":5001,\"type\":\"host.request\","
                    + "\"operationId\":7,\"capability\":\"host.smoke.echo\","
                    + "\"params\":{\"m\":1}}");
            CommandResult res = rt.sendAndAwait("runtime.ping", "{}", 2000L);
            assertTrue(res.isSuccess());
            boolean replied = false;
            for (String s : transport.sent) {
                if (s.contains("\"method\":\"host.complete\"") && s.contains("\"operationId\":7")) {
                    replied = true;
                }
            }
            assertTrue(replied);
        } finally {
            rt.stop();
        }
    }

    @SuppressWarnings("unchecked")
    private static long requestIdOf(String commandJson) {
        Object root = Json.parse(commandJson);
        return ((Number) ((java.util.Map<String, Object>) root).get("requestId")).longValue();
    }

    private static final class ScriptedTransport implements HostTransport {
        final Deque<String> events = new ArrayDeque<>();
        final List<String> sent = new ArrayList<>();
        volatile java.util.function.Consumer<String> onSend = cmd -> {};

        synchronized void enqueue(String e) {
            events.addLast(e);
            notifyAll();
        }

        void enqueueAfterDelay(String e) {
            new Thread(() -> {
                try { Thread.sleep(10); } catch (InterruptedException ie) {
                    Thread.currentThread().interrupt(); return;
                }
                enqueue(e);
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
