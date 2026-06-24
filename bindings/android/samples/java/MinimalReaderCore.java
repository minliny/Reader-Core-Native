package com.reader.core.samples;

import com.reader.core.ReaderCoreRuntime;
import java.util.concurrent.TimeUnit;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

public final class MinimalReaderCore {
    private static final Pattern OPERATION_ID =
            Pattern.compile("\"operationId\"\\s*:\\s*(\\d+)");

    private MinimalReaderCore() {
    }

    public static String ping() {
        try (ReaderCoreRuntime runtime = new ReaderCoreRuntime("{}")) {
            runtime.sendCommand("runtime.ping", 1L, "{}");
            return runtime.pollEvent(1, TimeUnit.SECONDS);
        }
    }

    public static String hostCompleteLoop() {
        try (ReaderCoreRuntime runtime = new ReaderCoreRuntime("{}")) {
            runtime.sendCommand(
                    "runtime.hostSmoke",
                    10L,
                    "{\"capability\":\"host.smoke.echo\",\"params\":{\"message\":\"android\"}}");

            String hostRequest = runtime.pollEvent(1, TimeUnit.SECONDS);
            if (hostRequest == null) {
                throw new IllegalStateException("host.request timed out");
            }

            long operationId = operationIdFrom(hostRequest);
            runtime.sendHostComplete(operationId, "{\"status\":\"ok\",\"source\":\"android\"}", 11L);
            return runtime.pollEvent(1, TimeUnit.SECONDS);
        }
    }

    private static long operationIdFrom(String eventJson) {
        Matcher matcher = OPERATION_ID.matcher(eventJson);
        if (!matcher.find()) {
            throw new IllegalStateException("host.request missing operationId: " + eventJson);
        }
        return Long.parseLong(matcher.group(1));
    }
}
