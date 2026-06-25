package com.reader.core.host;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;

/**
 * Contract tests for {@link HostReplyCodec} against the protocol conformance
 * fixtures. Each encoded command must match the corresponding fixture byte-for-byte
 * in canonical form, proving the adapter emits protocol-conformant
 * {@code host.complete} / {@code host.error} commands.
 */
class HostReplyCodecTest {

    @Test
    void encodeCompleteMatchesFixture() {
        String actual = HostReplyCodec.encodeComplete(
                302L, 1L, "{\"status\":\"ok\",\"source\":\"conformance\"}");
        assertEquals(canonical(fixture("complete.json")), canonical(actual));
    }

    @Test
    void encodeErrorMatchesFixture() {
        String actual = HostReplyCodec.encodeError(
                303L, 1L, "INTERNAL", "host conformance failure", true);
        assertEquals(canonical(fixture("error.json")), canonical(actual));
    }

    @Test
    void encodeHttpCompleteWithMetadataMatchesFixture() {
        String result = "{\"status\":200,"
                + "\"headers\":{\"content-type\":\"application/json\"},"
                + "\"body\":\"{\\\"books\\\":[]}\"}";
        String actual = HostReplyCodec.encodeComplete(502L, 1L, result);
        assertEquals(canonical(fixture("http-complete-with-metadata.json")), canonical(actual));
    }

    @Test
    void encodeCompleteRejectsOperationIdZero() {
        assertThrows(IllegalArgumentException.class,
                () -> HostReplyCodec.encodeComplete(1L, 0L, "{}"));
    }

    @Test
    void encodeErrorRejectsOperationIdZero() {
        assertThrows(IllegalArgumentException.class,
                () -> HostReplyCodec.encodeError(1L, 0L, "INTERNAL", "x", false));
    }

    @Test
    void encodeCompleteRejectsNonObjectResult() {
        assertThrows(IllegalArgumentException.class,
                () -> HostReplyCodec.encodeComplete(1L, 1L, "[1,2,3]"));
    }

    @Test
    void encodeViaReplyDispatchesCompleteAndError() {
        String complete = HostReplyCodec.encode(302L, HostReply.complete("{\"status\":\"ok\"}"), 1L);
        assertEquals(canonical("{\"protocolVersion\":1,\"requestId\":302,"
                + "\"method\":\"host.complete\",\"params\":{\"operationId\":1,"
                + "\"result\":{\"status\":\"ok\"}}}"), canonical(complete));

        String error = HostReplyCodec.encode(303L,
                HostReply.error("INTERNAL", "boom", true), 1L);
        assertEquals(canonical("{\"protocolVersion\":1,\"requestId\":303,"
                + "\"method\":\"host.error\",\"params\":{\"operationId\":1,"
                + "\"error\":{\"code\":\"INTERNAL\",\"message\":\"boom\",\"retryable\":true}}}"),
                canonical(error));
    }

    private static String canonical(String json) {
        return Json.canonicalize(json);
    }

    private static String fixture(String name) {
        try (InputStream in = HostReplyCodecTest.class
                .getResourceAsStream("/conformance/host/" + name)) {
            if (in == null) {
                throw new AssertionError("missing fixture: " + name);
            }
            return new String(in.readAllBytes(), StandardCharsets.UTF_8);
        } catch (IOException e) {
            throw new AssertionError("failed reading fixture " + name, e);
        }
    }
}
