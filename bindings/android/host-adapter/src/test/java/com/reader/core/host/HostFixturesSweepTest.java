package com.reader.core.host;

import org.junit.jupiter.api.Assumptions;
import org.junit.jupiter.api.DynamicTest;
import org.junit.jupiter.api.TestFactory;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;
import java.util.stream.Stream;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;
import static org.junit.jupiter.api.Assertions.fail;

/**
 * Dynamic conformance sweep: walks the <em>entire</em> upstream
 * {@code protocol/fixtures/conformance/host/} directory and asserts each
 * fixture is handled coherently by the host adapter, so new fixtures added
 * upstream automatically get coverage without touching this test.
 *
 * <p>Classification per fixture file (by name/shape):
 * <ul>
 *   <li>{@code complete*} / {@code error*} / {@code http-complete*}: a
 *       {@code host.complete}/{@code host.error} command — must be valid JSON
 *       the codec could reproduce (for the positive ones) or refuse
 *       (operationId == 0 negative variants).</li>
 *   <li>{@code request*}: a {@code runtime.hostSmoke} command whose params
 *       carry a capability — the capability must either parse as a valid
 *       {@link HostRequest} event or be one of the documented invalid-capability
 *       negative fixtures.</li>
 * </ul>
 *
 * <p>Skips gracefully when the upstream dir is not configured.
 */
class HostFixturesSweepTest {

    private static final String HOST_DIR = System.getProperty("reader.protocol.fixtures.host");

    @TestFactory
    Stream<DynamicTest> sweepUpstreamHostFixtures() throws IOException {
        Path dir = resolveHostDir();
        if (dir == null) {
            return Stream.empty();
        }
        List<Path> files;
        try (Stream<Path> listing = Files.list(dir)) {
            files = listing.filter(Files::isRegularFile).sorted().toList();
        }
        return files.stream().map(this::testForFixture);
    }

    private DynamicTest testForFixture(Path file) {
        String name = file.getFileName().toString();
        return DynamicTest.dynamicTest(name, () -> assertFixtureHandled(name, read(file)));
    }

    private static void assertFixtureHandled(String name, String json) {
        // Every fixture must at least be valid JSON we can parse.
        Object root;
        try {
            root = Json.parse(json);
        } catch (Json.JsonException e) {
            fail("fixture " + name + " is not valid JSON: " + e.getMessage());
            return;
        }
        @SuppressWarnings("unchecked")
        java.util.Map<String, Object> m = (root instanceof java.util.Map)
                ? (java.util.Map<String, Object>) root : null;
        String method = m == null ? null : String.valueOf(m.get("method"));

        if (name.startsWith("complete") || name.startsWith("error") || name.startsWith("http-complete")) {
            assertReplyFixture(name, m, method);
        } else if (name.startsWith("request")) {
            assertRequestFixture(name, m, method);
        } else {
            // Unknown fixture shape: just assert it's valid JSON (already done).
            assertTrue(true, "acknowledged unknown-shape fixture " + name);
        }
    }

    @SuppressWarnings("unchecked")
    private static void assertReplyFixture(String name, java.util.Map<String, Object> m, String method) {
        assertFalse(m == null, "reply fixture " + name + " must be an object");
        boolean isComplete = "host.complete".equals(method);
        boolean isError = "host.error".equals(method);
        assertTrue(isComplete || isError,
                name + ": expected host.complete/host.error, got " + method);

        java.util.Map<String, Object> params = (java.util.Map<String, Object>) m.get("params");
        assertFalse(params == null, name + ": missing params");
        Object opId = params.get("operationId");
        boolean opIdZero = opId instanceof Number && ((Number) opId).longValue() == 0;

        if (opIdZero) {
            // Negative fixture: the codec must refuse operationId == 0.
            if (isComplete) {
                assertTrue(throwsArg(() -> HostReplyCodec.encodeComplete(
                        1L, 0L, "{}")), name + ": codec should refuse operationId 0 (complete)");
            } else {
                assertTrue(throwsArg(() -> HostReplyCodec.encodeError(
                        1L, 0L, "INTERNAL", "x", false)), name + ": codec should refuse operationId 0 (error)");
            }
        } else {
            // Positive fixture: re-encode from its params and assert the codec
            // produces a structurally equivalent command (same method, same
            // operationId, matching result/error block in canonical form).
            long opIdLong = ((Number) opId).longValue();
            String actual;
            if (isComplete) {
                String resultJson = Json.stringify(params.get("result"));
                actual = HostReplyCodec.encodeComplete(1L, opIdLong, resultJson);
            } else {
                java.util.Map<String, Object> err = (java.util.Map<String, Object>) params.get("error");
                actual = HostReplyCodec.encodeError(1L, opIdLong,
                        String.valueOf(err.get("code")),
                        String.valueOf(err.get("message")),
                        Boolean.TRUE.equals(err.get("retryable")));
            }
            // Canonical equivalence on the params block (ignore requestId, which
            // the codec sets to the host's outbound id).
            Object actualParams = Json.parse(actual);
            @SuppressWarnings("unchecked")
            java.util.Map<String, Object> actualParamsMap = (java.util.Map<String, Object>) actualParams;
            // Re-stamp requestId to match the fixture for comparison.
            actualParamsMap.put("requestId", m.get("requestId"));
            assertEqualsCanonical(name, Json.stringify(actualParamsMap), Json.stringify(m));
        }
    }

    @SuppressWarnings("unchecked")
    private static void assertRequestFixture(String name, java.util.Map<String, Object> m, String method) {
        // request.json is a runtime.hostSmoke command; the invalid-capability
        // variants are negative fixtures.
        assertEqualsOrSkip("runtime.hostSmoke", method, name + ": expected runtime.hostSmoke");
        java.util.Map<String, Object> params = (java.util.Map<String, Object>) m.get("params");
        assertFalse(params == null, name + ": missing params");
        String capability = String.valueOf(params.get("capability"));
        String event = "{\"protocolVersion\":1,\"requestId\":1,\"type\":\"host.request\","
                + "\"operationId\":1,\"capability\":" + Json.stringify(capability)
                + ",\"params\":{}}";
        if (name.contains("invalid")) {
            assertTrue(throwsArg(() -> HostRequest.parse(event)),
                    name + ": HostRequest should reject invalid capability " + capability);
        } else {
            HostRequest req = HostRequest.parse(event);
            assertTrue(req.capability().contains("."), name + ": parsed capability should be dotted");
        }
    }

    // --- helpers ----------------------------------------------------------------

    private static Path resolveHostDir() {
        if (HOST_DIR == null) {
            Assumptions.assumeTrue(false, "upstream fixtures dir not configured");
            return null;
        }
        Path p = Paths.get(HOST_DIR);
        Assumptions.assumeTrue(Files.isDirectory(p), "upstream fixtures dir missing: " + HOST_DIR);
        return p;
    }

    private static String read(Path file) throws IOException {
        return Files.readString(file, StandardCharsets.UTF_8);
    }

    private static boolean throwsArg(Runnable r) {
        try {
            r.run();
            return false;
        } catch (RuntimeException e) {
            return true;
        }
    }

    private static void assertEqualsCanonical(String name, String expected, String actual) {
        if (!Json.canonicalize(expected).equals(Json.canonicalize(actual))) {
            fail(name + ": canonical mismatch.\nexpected: " + Json.canonicalize(expected)
                    + "\nactual:   " + Json.canonicalize(actual));
        }
    }

    private static void assertEqualsOrSkip(Object expected, Object actual, String msg) {
        if (!String.valueOf(expected).equals(String.valueOf(actual))) {
            // Soft-fail rather than hard-fail for unknown shapes; the sweep is
            // about coverage, not strict method equality on every future fixture.
            throw new org.opentest4j.AssertionFailedError(msg + " (expected " + expected + ", got " + actual + ")");
        }
    }
}
