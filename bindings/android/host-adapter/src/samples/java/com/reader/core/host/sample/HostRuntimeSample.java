package com.reader.core.host.sample;

import com.reader.core.ReaderCoreRuntime;
import com.reader.core.host.CommandResult;
import com.reader.core.host.Credential;
import com.reader.core.host.CredentialProvider;
import com.reader.core.host.CredentialResolveHandler;
import com.reader.core.host.HostRuntime;
import com.reader.core.host.HostSmokeEchoHandler;
import com.reader.core.host.HttpExecuteHandler;
import com.reader.core.host.HttpFetch;
import com.reader.core.host.HttpRequest;
import com.reader.core.host.HttpResponse;
import com.reader.core.host.ReaderCoreHostTransport;

import java.util.Collections;

/**
 * Runnable wiring sample for the unified {@link HostRuntime} facade — the
 * recommended production shape when a host app both answers Core's
 * {@code host.request}s and sends its own commands. One poll thread handles
 * both directions.
 *
 * <p>Like {@link HostBusSample}, this is <b>not unit-tested</b> (running needs
 * the JNI {@code .so} via {@code System.loadLibrary} in {@link ReaderCoreRuntime}'s
 * static initializer); it is compile-gated by the Gradle {@code compileSample}
 * task, which type-checks the full wiring under a plain JVM.
 *
 * <p>Run (with the {@code .so} on {@code java.library.path}):
 * <pre>
 *   java -cp ... -Djava.library.path=&lt;dir-with-libreader_core_jni.so&gt; \
 *        com.reader.core.host.sample.HostRuntimeSample
 * </pre>
 */
public final class HostRuntimeSample {

    public static void main(String[] args) {
        // 1. Core runtime via the existing JNI wrapper (rc_runtime_create).
        ReaderCoreRuntime runtime = new ReaderCoreRuntime("{}");

        // 2. Bridge to the host-adapter transport abstraction.
        ReaderCoreHostTransport transport = new ReaderCoreHostTransport(runtime);

        // 3. Unified facade: one poll thread routes host.request -> adapter
        //    and result/error -> sendAndAwait futures.
        HostRuntime rt = HostRuntime.over(transport)
                .register(HostSmokeEchoHandler.CAPABILITY, new HostSmokeEchoHandler())
                .register(HttpExecuteHandler.CAPABILITY,
                        new HttpExecuteHandler(new SampleHttpFetch()))
                .register(CredentialResolveHandler.CAPABILITY,
                        new CredentialResolveHandler(new SampleCredentialProvider()))
                .start();

        try {
            // 4. Host-initiated command: send runtime.ping and await its result.
            CommandResult ping = rt.sendAndAwait("runtime.ping", "{}", 5000L);
            if (ping.isSuccess()) {
                System.out.println("runtime.ping -> " + ping.dataJson());
            } else if (ping.isError()) {
                System.err.println("runtime.ping error -> " + ping.errorJson());
            }
            // Meanwhile, Core may issue host.request events (http.execute /
            // credential.resolve) that the poll thread answers via the
            // registered handlers — no extra wiring needed.
        } finally {
            rt.stop();
            runtime.close();
        }
    }

    static final class SampleHttpFetch implements HttpFetch {
        @Override
        public HttpResponse fetch(HttpRequest request) {
            return new HttpResponse(200, "", Collections.emptyMap());
        }
    }

    static final class SampleCredentialProvider implements CredentialProvider {
        @Override
        public Credential resolve(String credentialHandle) {
            return null;
        }
    }

    private HostRuntimeSample() {
    }
}
