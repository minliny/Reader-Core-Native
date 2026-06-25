package com.reader.core.host;

import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.TimeoutException;

/**
 * Unified, concurrency-safe facade over both halves of the Core protocol on a
 * single transport. Owns <em>one</em> poll thread that demultiplexes every
 * event off the transport:
 * <ul>
 *   <li>{@code host.request} → {@link HostAdapter#dispatch} → reply via
 *       {@link HostTransport#sendCommand} (the {@link HostEventLoop} half).</li>
 *   <li>{@code result} / {@code error} → complete the matching pending
 *       {@link CompletableFuture} from {@link #sendAndAwait} (the
 *       {@link HostCommander} half).</li>
 * </ul>
 *
 * <p>This resolves the demultiplexing caveat documented on {@link HostCommander}:
 * instead of the loop and commander competing for events, one poll site routes
 * every event by {@code type}. A host app wires this as its single Core entry
 * point:
 * <pre>
 *   HostRuntime rt = HostRuntime.over(transport)
 *       .register("http.execute", new HttpExecuteHandler(fetch))
 *       .start();
 *   CommandResult res = rt.sendAndAwait("runtime.ping", "{}", 1000);
 *   rt.stop();
 * </pre>
 *
 * <p>Pure-JVM: depends only on {@link HostTransport}, so unit-testable with a
 * fake (no NDK). The production transport is {@link ReaderCoreHostTransport}.
 */
public final class HostRuntime {

    private final HostTransport transport;
    private final HostAdapter adapter;
    private final long tickTimeoutMillis;
    private final ConcurrentHashMap<Long, CompletableFuture<CommandResult>> pending =
            new ConcurrentHashMap<>();
    private long counter;
    private Thread worker;
    private volatile boolean running;

    private HostRuntime(HostTransport transport, HostAdapter adapter, long tickTimeoutMillis, long firstRequestId) {
        this.transport = transport;
        this.adapter = adapter;
        this.tickTimeoutMillis = tickTimeoutMillis;
        this.counter = firstRequestId;
    }

    public static HostRuntime over(HostTransport transport) {
        return new HostRuntime(transport, new HostAdapter(), 1000L, 2000L);
    }

    public static HostRuntime over(HostTransport transport, long tickTimeoutMillis) {
        return new HostRuntime(transport, new HostAdapter(), tickTimeoutMillis, 2000L);
    }

    public HostRuntime register(String capability, CapabilityHandler handler) {
        adapter.register(capability, handler);
        return this;
    }

    /**
     * Send a command and await its result/error event, routed by the runtime's
     * own poll thread. Blocks the caller until the matching event arrives or
     * {@code timeoutMillis} elapses.
     */
    public CommandResult sendAndAwait(String method, String paramsJson, long timeoutMillis) {
        long requestId;
        CompletableFuture<CommandResult> future;
        synchronized (this) {
            requestId = counter++;
            future = new CompletableFuture<>();
            pending.put(requestId, future);
        }
        String command = HostCommander.encodeCommand(requestId, method, paramsJson);
        try {
            transport.sendCommand(command);
        } catch (RuntimeException e) {
            pending.remove(requestId);
            return CommandResult.error(requestId,
                    "{\"code\":\"INTERNAL\",\"message\":\"send failed: "
                            + e.getMessage() + "\",\"retryable\":true}");
        }
        try {
            return future.get(timeoutMillis, TimeUnit.MILLISECONDS);
        } catch (TimeoutException e) {
            pending.remove(requestId);
            return CommandResult.timeout(requestId);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            pending.remove(requestId);
            return CommandResult.timeout(requestId);
        } catch (ExecutionException e) {
            pending.remove(requestId);
            Throwable cause = e.getCause();
            String msg = cause == null ? e.getMessage() : cause.getMessage();
            return CommandResult.error(requestId,
                    "{\"code\":\"INTERNAL\",\"message\":\"" + msg + "\",\"retryable\":true}");
        }
    }

    /** Start the single poll thread. Idempotent. */
    public synchronized HostRuntime start() {
        if (running) {
            return this;
        }
        running = true;
        worker = new Thread(this::runLoop, "reader-host-runtime");
        worker.setDaemon(true);
        worker.start();
        return this;
    }

    /** Stop the poll thread. Idempotent. */
    public synchronized void stop() {
        running = false;
        if (worker != null) {
            worker.interrupt();
            try {
                worker.join();
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
            }
            worker = null;
        }
    }

    public synchronized boolean isRunning() {
        return running;
    }

    public int pendingCount() {
        return pending.size();
    }

    private void runLoop() {
        while (running && !Thread.currentThread().isInterrupted()) {
            String event;
            try {
                event = transport.pollEventJson(tickTimeoutMillis);
            } catch (RuntimeException e) {
                if (!running) break;
                continue;
            }
            if (event == null) {
                continue;
            }
            route(event);
        }
    }

    @SuppressWarnings("unchecked")
    private void route(String eventJson) {
        Object root;
        try {
            root = Json.parse(eventJson);
        } catch (Json.JsonException e) {
            return;
        }
        if (!(root instanceof java.util.Map)) {
            return;
        }
        java.util.Map<String, Object> m = (java.util.Map<String, Object>) root;
        Object type = m.get("type");
        if ("host.request".equals(type)) {
            HostRequest req;
            try {
                req = HostRequest.parse(eventJson);
            } catch (IllegalArgumentException e) {
                return;
            }
            HostReply reply = adapter.dispatch(req);
            // The runtime owns outbound reply requestIds.
            long replyRequestId;
            synchronized (this) {
                replyRequestId = counter++;
            }
            transport.sendCommand(HostReplyCodec.encode(replyRequestId, reply, req.operationId()));
        } else if ("result".equals(type) || "error".equals(type)) {
            Object idVal = m.get("requestId");
            if (idVal instanceof Number) {
                long id = ((Number) idVal).longValue();
                CompletableFuture<CommandResult> future = pending.remove(id);
                if (future != null) {
                    if ("result".equals(type)) {
                        Object data = m.get("data");
                        future.complete(CommandResult.success(id,
                                data == null ? "{}" : Json.stringify(data)));
                    } else {
                        Object err = m.get("error");
                        future.complete(CommandResult.error(id,
                                err == null ? "{}" : Json.stringify(err)));
                    }
                }
            }
        }
    }
}
