package com.reader.core.host;

import org.junit.jupiter.api.Assumptions;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.Map;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

/**
 * Protocol conformance tests that read the <em>upstream</em> fixture corpus
 * directly from {@code protocol/fixtures/conformance/host/} (the protocol's
 * source of truth) rather than copied fixtures, so a protocol change breaks
 * this test. The host adapter's wire output must match each positive reply
 * fixture byte-for-byte in canonical form (modulo the host-chosen outbound
 * {@code requestId}), and must refuse each negative (operationId == 0) fixture.
 *
 * <p>If the {@code reader.protocol.fixtures.host} system property is unset or
 * the directory is absent (e.g. running the jar outside the repo), these tests
 * are skipped rather than failed, so the copied-fixture tests still carry the
 * contract.
 */
class ProtocolConformanceTest {

    private static final String HOST_DIR = System.getProperty("reader.protocol.fixtures.host");

    private static Path fixturePath(String name) {
        Assumptions.assumeTrue(HOST_DIR != null, "upstream fixtures dir not configured");
        Path p = Paths.get(HOST_DIR, name);
        Assumptions.assumeTrue(Files.exists(p), "upstream fixture missing: " + name);
        return p;
    }

    private static String readFixture(String name) {
        try {
            return Files.readString(fixturePath(name), StandardCharsets.UTF_8);
        } catch (IOException e) {
            throw new AssertionError("failed reading upstream fixture " + name, e);
        }
    }

    @Test
    void encodeCompleteMatchesUpstreamFixture() {
        String actual = HostReplyCodec.encodeComplete(
                302L, 1L, "{\"status\":\"ok\",\"source\":\"conformance\"}");
        assertEquals(Json.canonicalize(readFixture("complete.json")), Json.canonicalize(actual));
    }

    @Test
    void encodeErrorMatchesUpstreamFixture() {
        String actual = HostReplyCodec.encodeError(
                303L, 1L, "INTERNAL", "host conformance failure", true);
        assertEquals(Json.canonicalize(readFixture("error.json")), Json.canonicalize(actual));
    }

    @Test
    void encodeHttpCompleteWithMetadataMatchesUpstreamFixture() {
        String result = "{\"status\":200,"
                + "\"headers\":{\"content-type\":\"application/json\"},"
                + "\"body\":\"{\\\"books\\\":[]}\"}";
        String actual = HostReplyCodec.encodeComplete(502L, 1L, result);
        assertEquals(Json.canonicalize(readFixture("http-complete-with-metadata.json")),
                Json.canonicalize(actual));
    }

    @Test
    void encodeHttpCompleteInvalidStatusMatchesUpstreamFixture() {
        // status 99 is a non-HTTP status; the adapter passes it through verbatim,
        // matching the protocol's negative-status conformance fixture.
        String result = "{\"status\":99,\"body\":\"{\\\"books\\\":[]}\"}";
        String actual = HostReplyCodec.encodeComplete(505L, 1L, result);
        assertEquals(Json.canonicalize(readFixture("http-complete-invalid-status.json")),
                Json.canonicalize(actual));
    }

    @Test
    void codecRefusesOperationZeroCompleteFixture() {
        // complete-operation-zero.json is a negative fixture: operationId == 0
        // is forbidden by the event schema. The codec must refuse to emit it.
        assertThrows(IllegalArgumentException.class,
                () -> HostReplyCodec.encodeComplete(305L, 0L, "{\"status\":\"invalid\"}"));
    }

    @Test
    void codecRefusesOperationZeroErrorFixture() {
        assertThrows(IllegalArgumentException.class,
                () -> HostReplyCodec.encodeError(306L, 0L, "INTERNAL", "invalid operation id", false));
    }

    @Test
    void hostRequestAcceptsSmokeRequestParamsShape() {
        // request.json is a runtime.hostSmoke command whose params carry the
        // capability + nested params. Validate HostRequest.parse accepts the
        // equivalent host.request event shape built from those params.
        String requestFixture = readFixture("request.json");
        Object root = Json.parse(requestFixture);
        @SuppressWarnings("unchecked")
        Map<String, Object> params = (Map<String, Object>) ((Map<String, Object>) root).get("params");
        String capability = (String) params.get("capability");
        String nestedParams = Json.stringify(params.get("params"));

        HostRequest req = HostRequest.parse("{\"protocolVersion\":1,\"requestId\":301,"
                + "\"type\":\"host.request\",\"operationId\":1,"
                + "\"capability\":\"" + capability + "\","
                + "\"params\":" + nestedParams + "}");
        assertEquals("host.smoke.echo", req.capability());
        assertEquals(nestedParams, req.paramsJson());

        // The smoke echo handler must round-trip the params verbatim.
        HostSmokeEchoHandler handler = new HostSmokeEchoHandler();
        HostReply reply = handler.handle(req);
        assertTrue(reply.isComplete());
        assertEquals(Json.canonicalize(nestedParams),
                Json.canonicalize(((HostReply.Complete) reply).resultJson()));
    }

    @Test
    void hostRequestRejectsInvalidCapabilityFixtures() {
        // request-invalid-capability-* are negative fixtures: the adapter must
        // reject these capability strings at parse time.
        assertThrows(IllegalArgumentException.class, () ->
                HostRequest.parse(eventWithCapability(readCapability("request-invalid-capability-empty-segment.json"))));
        assertThrows(IllegalArgumentException.class, () ->
                HostRequest.parse(eventWithCapability(readCapability("request-invalid-capability-whitespace.json"))));
    }

    @SuppressWarnings("unchecked")
    private static String readCapability(String fixtureName) {
        Object root = Json.parse(readFixture(fixtureName));
        return (String) ((Map<String, Object>) ((Map<String, Object>) root).get("params")).get("capability");
    }

    private static String eventWithCapability(String capability) {
        return "{\"protocolVersion\":1,\"requestId\":1,\"type\":\"host.request\","
                + "\"operationId\":1,\"capability\":" + Json.stringify(capability)
                + ",\"params\":{}}";
    }
}
