package com.reader.core.host.sample;

import com.reader.core.ReaderCoreRuntime;
import com.reader.core.host.CredentialResolveHandler;
import com.reader.core.host.CredentialProvider;
import com.reader.core.host.Credential;
import com.reader.core.host.HostBus;
import com.reader.core.host.HttpExecuteHandler;
import com.reader.core.host.HttpFetch;
import com.reader.core.host.HttpRequest;
import com.reader.core.host.HttpResponse;
import com.reader.core.host.HostSmokeEchoHandler;
import com.reader.core.host.ReaderCoreHostTransport;

import java.util.Collections;

/**
 * End-to-end wiring sample: binds {@link HostBus} to the real C ABI surface via
 * {@link ReaderCoreHostTransport} and registers the three shipped capability
 * handlers ({@code http.execute}, {@code host.smoke.echo},
 * {@code credential.resolve}). This is the concrete access path an Android
 * host app would copy.
 *
 * <p><b>Not unit-tested</b>: running it requires the JNI shared library
 * ({@code System.loadLibrary("reader_core_jni")} in {@link ReaderCoreRuntime}'s
 * static initializer), so it only executes on a device/emulator with the
 * {@code .so} present. It is compile-gated instead — the Gradle {@code
 * compileSample} task type-checks the full wiring under a plain JVM, proving the
 * host-adapter module, the JNI wrapper, and the capability handlers wire
 * together without touching Native or other platforms.
 *
 * <p>Run (with the {@code .so} on {@code java.library.path}):
 * <pre>
 *   java -cp ... -Djava.library.path=&lt;dir-with-libreader_core_jni.so&gt; \
 *        com.reader.core.host.sample.HostBusSample
 * </pre>
 */
public final class HostBusSample {

    public static void main(String[] args) {
        // 1. Create the Core runtime via the existing JNI wrapper (rc_runtime_create).
        ReaderCoreRuntime runtime = new ReaderCoreRuntime("{}");

        // 2. Bridge the runtime to the host-adapter transport abstraction.
        ReaderCoreHostTransport transport = new ReaderCoreHostTransport(runtime);

        // 3. Assemble the host bus and register platform capabilities.
        HostBus bus = HostBus.over(transport)
                .register(HostSmokeEchoHandler.CAPABILITY, new HostSmokeEchoHandler())
                .register(HttpExecuteHandler.CAPABILITY,
                        new HttpExecuteHandler(new SampleHttpFetch()))
                .register(CredentialResolveHandler.CAPABILITY,
                        new CredentialResolveHandler(new SampleCredentialProvider()));

        // 4. Drive the loop on a daemon thread. The bus polls host.request
        //    events off rc_event_callback's queue and replies via rc_runtime_send.
        bus.start();

        // ... host app runs, Core issues host.request events, bus answers them ...

        // 5. On shutdown, stop the loop and close the runtime.
        Runtime.getRuntime().addShutdownHook(new Thread(() -> {
            bus.stop();
            runtime.close();
        }));
    }

    /** Sample host-owned HTTP fetch: a real app plugs in OkHttp / Cronet here. */
    static final class SampleHttpFetch implements HttpFetch {
        @Override
        public HttpResponse fetch(HttpRequest request) {
            // Placeholder — a real host performs the TLS fetch here.
            return new HttpResponse(200, "", Collections.emptyMap());
        }
    }

    /** Sample host-owned credential store: a real app plugs in Keystore here. */
    static final class SampleCredentialProvider implements CredentialProvider {
        @Override
        public Credential resolve(String credentialHandle) {
            // Placeholder — a real host reads from Keychain/Keystore here.
            return null;
        }
    }

    private HostBusSample() {
    }
}
