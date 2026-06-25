package com.reader.core.host;

import java.util.Map;

/**
 * The host-side event loop that closes the Core ↔ host access path.
 *
 * <p>Each {@link #tick()} polls one event off the transport, filters for
 * {@code host.request}, dispatches it through {@link HostAdapter}, encodes the
 * reply via {@link HostReplyCodec}, and sends it back on the transport. Non-
 * {@code host.request} events (results, errors) are ignored by this loop —
 * they belong to the host's command-response correlation, not capability
 * fulfillment.
 *
 * <p>The loop owns the outbound command {@code requestId} (a monotonic counter
 * starting at {@link #FIRST_OUTBOUND_REQUEST_ID}); the {@code operationId} is
 * taken from the parsed request. This matches the protocol: a host reply is a
 * fresh command whose {@code requestId} is the host's, while
 * {@code operationId} correlates to Core's blocked request.
 *
 * <p>Pure-JVM: depends only on {@link HostTransport}, so it is unit-testable
 * with a fake transport and no NDK library.
 */
public final class HostEventLoop {

    private static final long FIRST_OUTBOUND_REQUEST_ID = 1000L;
    private static final long DEFAULT_TICK_TIMEOUT_MS = 1000L;

    private final HostTransport transport;
    private final HostAdapter adapter;
    private final long tickTimeoutMillis;
    private long nextOutboundRequestId = FIRST_OUTBOUND_REQUEST_ID;
    private int processed;
    private int replied;
    private int skipped;

    public HostEventLoop(HostTransport transport, HostAdapter adapter) {
        this(transport, adapter, DEFAULT_TICK_TIMEOUT_MS);
    }

    public HostEventLoop(HostTransport transport, HostAdapter adapter, long tickTimeoutMillis) {
        if (transport == null) throw new IllegalArgumentException("transport required");
        if (adapter == null) throw new IllegalArgumentException("adapter required");
        if (tickTimeoutMillis < 0) throw new IllegalArgumentException("negative timeout");
        this.transport = transport;
        this.adapter = adapter;
        this.tickTimeoutMillis = tickTimeoutMillis;
    }

    /**
     * Process at most one event. Returns {@code true} if a {@code host.request}
     * was dispatched and a reply was sent; {@code false} if the poll timed out
     * or the event was not a host request (or was malformed).
     */
    public synchronized boolean tick() {
        String eventJson = transport.pollEventJson(tickTimeoutMillis);
        if (eventJson == null) {
            return false;
        }
        processed++;
        String type = eventType(eventJson);
        if (!"host.request".equals(type)) {
            skipped++;
            return false;
        }
        HostRequest request;
        try {
            request = HostRequest.parse(eventJson);
        } catch (IllegalArgumentException e) {
            // Malformed host.request: no reliable operationId to reply to.
            skipped++;
            return false;
        }
        HostReply reply = adapter.dispatch(request);
        long outboundRequestId = nextOutboundRequestId++;
        String command = HostReplyCodec.encode(outboundRequestId, reply, request.operationId());
        transport.sendCommand(command);
        replied++;
        return true;
    }

    /** Run until {@link #tick()} returns false (timeout or non-host event). */
    public synchronized void drain() {
        while (tick()) {
            // keep draining host.request events
        }
    }

    public synchronized int processedCount() {
        return processed;
    }

    public synchronized int repliedCount() {
        return replied;
    }

    public synchronized int skippedCount() {
        return skipped;
    }

    @SuppressWarnings("unchecked")
    private static String eventType(String eventJson) {
        Object root;
        try {
            root = Json.parse(eventJson);
        } catch (Json.JsonException e) {
            return null;
        }
        if (!(root instanceof Map)) {
            return null;
        }
        Object t = ((Map<String, Object>) root).get("type");
        return t == null ? null : t.toString();
    }
}
