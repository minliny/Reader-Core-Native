package com.reader.core.host;

/**
 * Transport abstraction over the Core runtime's event poll and command send
 * surfaces. The default production implementation wraps
 * {@code ReaderCoreRuntime} (JNI → {@code rc_runtime_send} /
 * {@code rc_event_callback}); tests inject a fake. This keeps the
 * {@link HostEventLoop} pure-JVM and unit-testable without the NDK library.
 *
 * <p>Implementations must be thread-safe if the loop is driven concurrently.
 */
public interface HostTransport {

    /**
     * Block up to {@code timeoutMillis} for the next Core event, returning its
     * JSON string, or {@code null} on timeout. A non-null return need not be a
     * {@code host.request} event; the loop filters by {@code type}.
     */
    String pollEventJson(long timeoutMillis);

    /** Send an already-encoded Core command JSON (e.g. {@code host.complete}). */
    void sendCommand(String commandJson);
}
