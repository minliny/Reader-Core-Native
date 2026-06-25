package com.reader.core.host;

import java.util.LinkedHashMap;
import java.util.Map;

/**
 * Host-side adapter that bridges Core {@code host.request} events to
 * platform-owned capabilities.
 *
 * <p>The adapter is a pure-JVM component: it parses nothing off the wire here
 * (callers pass a parsed {@link HostRequest}), dispatches to a registered
 * {@link CapabilityHandler} by capability string, and returns a
 * {@link HostReply} that the caller encodes via {@link HostReplyCodec} and
 * sends back to Core through the existing C ABI surface
 * ({@code rc_runtime_send}). It therefore consumes the Core ABI/protocol
 * without extending it.
 *
 * <p>Failure modes map to {@code host.error}:
 * <ul>
 *   <li>no handler registered for the capability → non-retryable INTERNAL</li>
 *   <li>handler returns null → non-retryable INTERNAL</li>
 *   <li>handler throws → retryable INTERNAL (transient host failure)</li>
 * </ul>
 */
public final class HostAdapter {

    private static final String INTERNAL = "INTERNAL";

    private final Map<String, CapabilityHandler> handlers = new LinkedHashMap<>();

    public synchronized void register(String capability, CapabilityHandler handler) {
        if (capability == null || capability.isEmpty()) {
            throw new IllegalArgumentException("capability required");
        }
        if (handler == null) {
            throw new IllegalArgumentException("handler required");
        }
        handlers.put(capability, handler);
    }

    public synchronized boolean isRegistered(String capability) {
        return handlers.containsKey(capability);
    }

    public synchronized HostReply dispatch(HostRequest request) {
        if (request == null) {
            throw new IllegalArgumentException("request required");
        }
        CapabilityHandler handler = handlers.get(request.capability());
        if (handler == null) {
            return HostReply.error(INTERNAL,
                    "unsupported capability: " + request.capability(), false);
        }
        try {
            HostReply reply = handler.handle(request);
            if (reply == null) {
                return HostReply.error(INTERNAL,
                        "handler returned null for " + request.capability(), false);
            }
            return reply;
        } catch (RuntimeException e) {
            return HostReply.error(INTERNAL,
                    "handler threw for " + request.capability() + ": " + e.getMessage(), true);
        }
    }
}
