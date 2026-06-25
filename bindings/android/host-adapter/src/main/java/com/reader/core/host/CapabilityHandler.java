package com.reader.core.host;

/**
 * Host-side handler for one Core capability (e.g. {@code http.execute},
 * {@code host.smoke.echo}). Implementations perform the platform operation
 * and return a {@link HostReply}; the adapter encodes it as a
 * {@code host.complete} / {@code host.error} command to send back via
 * {@code rc_runtime_send}.
 *
 * <p>Handlers run on whatever thread dispatches the request. They must be
 * thread-safe if the host adapter is driven concurrently.
 */
public interface CapabilityHandler {

    HostReply handle(HostRequest request);
}
