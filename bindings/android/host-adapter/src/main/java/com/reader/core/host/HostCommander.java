package com.reader.core.host;

/**
 * The host side of the protocol's command/response half. While
 * {@link HostEventLoop} answers Core's {@code host.request} events, a host app
 * also <em>sends</em> commands to Core (e.g. {@code runtime.ping},
 * {@code book.search}) and must await the matching {@code result} or
 * {@code error} event by {@code requestId}.
 *
 * <p>{@link HostCommander} sends a command via the transport, then polls until
 * the event with the same {@code requestId} arrives, returning a
 * {@link CommandResult}. It is the correlate of {@link HostEventLoop} for
 * host-initiated commands.
 *
 * <p><b>Demultiplexing caveat.</b> If a {@link HostBus}/{@link HostEventLoop} is
 * driving the same transport concurrently, its {@code tick()} would consume
 * events this commander is waiting for (and vice versa). In a host app, drive
 * both through a single poll site: either let the commander own the poll and
 * hand {@code host.request} events to the loop, or run the commander only when
 * the loop is stopped. The pure-JVM tests use an exclusive fake transport.
 */
public final class HostCommander {

    private final HostTransport transport;
    private long counter;

    public HostCommander(HostTransport transport) {
        this(transport, 2000L);
    }

    public HostCommander(HostTransport transport, long firstRequestId) {
        if (transport == null) {
            throw new IllegalArgumentException("transport required");
        }
        this.transport = transport;
        this.counter = firstRequestId;
    }

    /**
     * Send a command and await its result/error event.
     *
     * @param method    Core command method, e.g. {@code "runtime.ping"}.
     * @param paramsJson command params object JSON (may be {@code "{}"}).
     * @param timeoutMillis max wait for the matching event.
     * @return the result or error event for this command's requestId.
     */
    public CommandResult sendAndAwait(String method, String paramsJson, long timeoutMillis) {
        long requestId = counter++;
        String command = encodeCommand(requestId, method, paramsJson);
        transport.sendCommand(command);
        long deadline = System.currentTimeMillis() + Math.max(timeoutMillis, 0);
        while (true) {
            long remaining = deadline - System.currentTimeMillis();
            if (remaining <= 0) {
                return CommandResult.timeout(requestId);
            }
            String event = transport.pollEventJson(remaining);
            if (event == null) {
                continue;
            }
            CommandResult matched = match(requestId, event);
            if (matched != null) {
                return matched;
            }
            // Not our event; a host app would re-route it (e.g. to the loop).
        }
    }

    public synchronized long nextRequestId() {
        return counter;
    }

    static String encodeCommand(long requestId, String method, String paramsJson) {
        return "{\"protocolVersion\":1,\"requestId\":" + requestId
                + ",\"method\":" + Json.stringify(method)
                + ",\"params\":" + (paramsJson == null || paramsJson.trim().isEmpty()
                        ? "{}" : paramsJson.trim())
                + "}";
    }

    @SuppressWarnings("unchecked")
    private static CommandResult match(long expectedRequestId, String eventJson) {
        Object root;
        try {
            root = Json.parse(eventJson);
        } catch (Json.JsonException e) {
            return null;
        }
        if (!(root instanceof java.util.Map)) {
            return null;
        }
        java.util.Map<String, Object> m = (java.util.Map<String, Object>) root;
        Object idVal = m.get("requestId");
        if (!(idVal instanceof Number) || ((Number) idVal).longValue() != expectedRequestId) {
            return null;
        }
        Object type = m.get("type");
        if ("result".equals(type)) {
            Object data = m.get("data");
            String dataJson = data == null ? "{}" : Json.stringify(data);
            return CommandResult.success(expectedRequestId, dataJson);
        }
        if ("error".equals(type)) {
            Object err = m.get("error");
            String errJson = err == null ? "{}" : Json.stringify(err);
            return CommandResult.error(expectedRequestId, errJson);
        }
        return null;
    }
}
