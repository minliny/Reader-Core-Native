package com.reader.core.host;

import org.junit.jupiter.api.Test;

import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Deque;
import java.util.List;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

/**
 * Full-stack integration test: {@link HostRuntime} wired with all three shipped
 * capability handlers ({@code http.execute}, {@code host.smoke.echo},
 * {@code credential.resolve}) processes a mixed event sequence — two
 * {@code host.request}s answered by the adapter and one host-initiated command
 * whose {@code result} is correlated by {@code sendAndAwait} — through the
 * single poll thread. Proves the adapter composes end-to-end.
 */
class HostRuntimeIntegrationTest {

    @Test
    void mixedHostRequestsAndCommandResultFlowThroughOnePollThread() throws Exception {
        ScriptedTransport transport = new ScriptedTransport();

        // When the host sends a command (runtime.ping), Core replies with a result.
        transport.onSend = cmd -> {
            if (cmd.contains("\"method\":\"runtime.ping\"")) {
                long id = requestIdOf(cmd);
                transport.enqueueAfterDelay("{\"protocolVersion\":1,\"requestId\":" + id
                        + ",\"type\":\"result\",\"data\":{\"pong\":true}}");
            }
        };

        HttpFetch fetch = req -> new HttpResponse(200, "books:" + req.url(),
                java.util.Collections.singletonMap("content-type", "text/plain"));
        CredentialProvider creds = handle -> "webdav-default".equals(handle)
                ? new Credential("alice", "s3cret") : null;

        HostRuntime rt = HostRuntime.over(transport, 50L)
                .register(HostSmokeEchoHandler.CAPABILITY, new HostSmokeEchoHandler())
                .register(HttpExecuteHandler.CAPABILITY, new HttpExecuteHandler(fetch))
                .register(CredentialResolveHandler.CAPABILITY,
                        new CredentialResolveHandler(creds))
                .start();
        try {
            // Pre-enqueue two host.request events the poll thread must answer.
            transport.enqueue("{\"protocolVersion\":1,\"requestId\":5001,\"type\":\"host.request\","
                    + "\"operationId\":11,\"capability\":\"http.execute\","
                    + "\"params\":{\"url\":\"https://x.test\",\"method\":\"GET\"}}");
            transport.enqueue("{\"protocolVersion\":1,\"requestId\":5002,\"type\":\"host.request\","
                    + "\"operationId\":12,\"capability\":\"credential.resolve\","
                    + "\"params\":{\"credentialHandle\":\"webdav-default\"}}");

            // Concurrently send a host-initiated command.
            CommandResult res = rt.sendAndAwait("runtime.ping", "{}", 2000L);
            assertTrue(res.isSuccess());
            assertEquals("{\"pong\":true}", res.dataJson());
        } finally {
            rt.stop();
        }

        // The runtime must have emitted host.complete replies for both requests.
        boolean httpReplied = false;
        boolean credReplied = false;
        for (String sent : transport.sent) {
            if (sent.contains("\"method\":\"host.complete\"") && sent.contains("\"operationId\":11")) {
                assertTrue(sent.contains("\"status\":200"));
                assertTrue(sent.contains("books:https://x.test"));
                httpReplied = true;
            }
            if (sent.contains("\"method\":\"host.complete\"") && sent.contains("\"operationId\":12")) {
                assertTrue(sent.contains("\"username\":\"alice\""));
                assertTrue(sent.contains("\"password\":\"s3cret\""));
                credReplied = true;
            }
        }
        assertTrue(httpReplied, "http.execute host.request should have been answered");
        assertTrue(credReplied, "credential.resolve host.request should have been answered");
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
