package com.reader.core.host;

/**
 * One-stop host-side integration point over the Core ABI/protocol access path.
 *
 * <p>Bundles a {@link HostTransport}, {@link HostAdapter}, and {@link HostEventLoop}
 * into the surface an Android host app actually wires up:
 * <pre>
 *   HostBus bus = HostBus.over(transport);
 *   bus.register("http.execute", new HttpExecuteHandler(okHttpFetch));
 *   bus.register("host.smoke.echo", new HostSmokeEchoHandler());
 *   bus.start();   // daemon poll thread drives tick() until stop()
 *   ...
 *   bus.stop();
 * </pre>
 *
 * <p>The bus does not touch the C ABI directly. It depends only on
 * {@link HostTransport}, so it is fully unit-testable with a fake transport and
 * no NDK library. The production transport is {@link ReaderCoreHostTransport}.
 *
 * <p>Threading: {@link #start()} spawns a daemon thread that repeatedly calls
 * {@link HostEventLoop#tick()} until {@link #stop()} interrupts it. The
 * underlying {@link HostAdapter} is synchronized, so handler registration and
 * dispatch remain consistent across the poll thread and the caller.
 */
public final class HostBus {

    private final HostTransport transport;
    private final HostAdapter adapter;
    private final HostEventLoop loop;
    private final long tickTimeoutMillis;

    private Thread worker;
    private volatile boolean running;

    private HostBus(HostTransport transport, HostAdapter adapter, HostEventLoop loop,
                    long tickTimeoutMillis) {
        this.transport = transport;
        this.adapter = adapter;
        this.loop = loop;
        this.tickTimeoutMillis = tickTimeoutMillis;
    }

    public static HostBus over(HostTransport transport) {
        return over(transport, 1000L);
    }

    public static HostBus over(HostTransport transport, long tickTimeoutMillis) {
        if (transport == null) {
            throw new IllegalArgumentException("transport required");
        }
        HostAdapter adapter = new HostAdapter();
        HostEventLoop loop = new HostEventLoop(transport, adapter, tickTimeoutMillis);
        return new HostBus(transport, adapter, loop, tickTimeoutMillis);
    }

    public HostBus register(String capability, CapabilityHandler handler) {
        adapter.register(capability, handler);
        return this;
    }

    public boolean isRegistered(String capability) {
        return adapter.isRegistered(capability);
    }

    /** Process at most one event on the caller's thread. */
    public boolean tick() {
        return loop.tick();
    }

    /** Drain pending host.request events until a non-host event or timeout. */
    public void drain() {
        loop.drain();
    }

    public int processedCount() {
        return loop.processedCount();
    }

    public int repliedCount() {
        return loop.repliedCount();
    }

    public int skippedCount() {
        return loop.skippedCount();
    }

    /** Exposed for tests that need to drive the transport directly. */
    HostTransport transport() {
        return transport;
    }

    /**
     * Start a daemon poll thread that repeatedly calls {@link #tick()} until
     * {@link #stop()}. Idempotent: a no-op if already running.
     */
    public synchronized void start() {
        if (running) {
            return;
        }
        running = true;
        worker = new Thread(this::runLoop, "reader-host-bus");
        worker.setDaemon(true);
        worker.start();
    }

    /** Stop the poll thread and wait for it to drain. Idempotent. */
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

    private void runLoop() {
        while (running && !Thread.currentThread().isInterrupted()) {
            try {
                loop.tick();
            } catch (RuntimeException e) {
                // A transient poll/send failure must not kill the bus; the
                // transport decides how long to block on the next tick.
                if (!running) {
                    break;
                }
            }
        }
    }
}
