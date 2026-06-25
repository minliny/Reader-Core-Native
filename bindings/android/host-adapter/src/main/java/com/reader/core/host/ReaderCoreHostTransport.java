package com.reader.core.host;

import com.reader.core.ReaderCoreRuntime;

/**
 * Adapts {@link ReaderCoreRuntime} (JNI → C ABI) to {@link HostTransport}.
 *
 * <p>This is the production wiring that closes the access path: events polled
 * via {@code rc_event_callback} → Java queue → {@code pollEvent}, and replies
 * sent via {@code rc_runtime_send}. It lives in the host-adapter module so the
 * pure-JVM {@link HostEventLoop} can be unit-tested against a fake transport,
 * while the real transport only depends on the existing JNI wrapper — no new
 * C ABI symbols.
 */
public final class ReaderCoreHostTransport implements HostTransport {

    private final ReaderCoreRuntime runtime;
    private final long pollTimeoutMillis;

    public ReaderCoreHostTransport(ReaderCoreRuntime runtime) {
        this(runtime, 1000L);
    }

    public ReaderCoreHostTransport(ReaderCoreRuntime runtime, long pollTimeoutMillis) {
        if (runtime == null) {
            throw new IllegalArgumentException("runtime required");
        }
        this.runtime = runtime;
        this.pollTimeoutMillis = pollTimeoutMillis;
    }

    @Override
    public String pollEventJson(long timeoutMillis) {
        return runtime.pollEvent(timeoutMillis, java.util.concurrent.TimeUnit.MILLISECONDS);
    }

    @Override
    public void sendCommand(String commandJson) {
        runtime.send(commandJson);
    }

    long pollTimeoutMillis() {
        return pollTimeoutMillis;
    }
}
