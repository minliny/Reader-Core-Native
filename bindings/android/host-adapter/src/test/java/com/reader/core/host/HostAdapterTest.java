package com.reader.core.host;

import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

/**
 * Contract tests for {@link HostAdapter}: dispatch of {@code host.request} to
 * capability handlers, and end-to-end encoding of the resulting reply against
 * the protocol conformance fixtures.
 */
class HostAdapterTest {

    @Test
    void dispatchesToRegisteredHandlerAndEncodesCompleteFixture() {
        HostAdapter adapter = new HostAdapter();
        adapter.register("host.smoke.echo", req ->
                HostReply.complete("{\"status\":\"ok\",\"source\":\"conformance\"}"));

        HostRequest req = HostRequest.parse("{\"protocolVersion\":1,\"requestId\":301,"
                + "\"type\":\"host.request\",\"operationId\":1,"
                + "\"capability\":\"host.smoke.echo\",\"params\":{}}");

        HostReply reply = adapter.dispatch(req);
        assertTrue(reply.isComplete());

        String command = HostReplyCodec.encode(302L, reply, req.operationId());
        assertEquals(Json.canonicalize(fixtureComplete()), Json.canonicalize(command));
    }

    @Test
    void dispatchesErrorAndEncodesErrorFixture() {
        HostAdapter adapter = new HostAdapter();
        adapter.register("host.smoke.fail", req ->
                HostReply.error("INTERNAL", "host conformance failure", true));

        HostRequest req = HostRequest.parse("{\"protocolVersion\":1,\"requestId\":301,"
                + "\"type\":\"host.request\",\"operationId\":1,"
                + "\"capability\":\"host.smoke.fail\",\"params\":{}}");

        HostReply reply = adapter.dispatch(req);
        assertTrue(reply.isError());

        String command = HostReplyCodec.encode(303L, reply, req.operationId());
        assertEquals(Json.canonicalize(fixtureError()), Json.canonicalize(command));
    }

    @Test
    void unsupportedCapabilityYieldsNonRetryableInternalError() {
        HostAdapter adapter = new HostAdapter();
        HostRequest req = HostRequest.parse("{\"protocolVersion\":1,\"requestId\":301,"
                + "\"type\":\"host.request\",\"operationId\":1,"
                + "\"capability\":\"host.unregistered\",\"params\":{}}");

        HostReply reply = adapter.dispatch(req);
        assertTrue(reply.isError());
        HostReply.Error err = (HostReply.Error) reply;
        assertEquals("INTERNAL", err.code());
        assertEquals(false, err.retryable());
    }

    @Test
    void handlerThrowingYieldsRetryableInternalError() {
        HostAdapter adapter = new HostAdapter();
        adapter.register("host.boom", req -> {
            throw new IllegalStateException("transient");
        });

        HostRequest req = HostRequest.parse("{\"protocolVersion\":1,\"requestId\":301,"
                + "\"type\":\"host.request\",\"operationId\":1,"
                + "\"capability\":\"host.boom\",\"params\":{}}");

        HostReply reply = adapter.dispatch(req);
        assertTrue(reply.isError());
        HostReply.Error err = (HostReply.Error) reply;
        assertEquals("INTERNAL", err.code());
        assertTrue(err.retryable());
    }

    @Test
    void handlerReturningNullYieldsNonRetryableInternalError() {
        HostAdapter adapter = new HostAdapter();
        adapter.register("host.null", req -> null);

        HostRequest req = HostRequest.parse("{\"protocolVersion\":1,\"requestId\":301,"
                + "\"type\":\"host.request\",\"operationId\":1,"
                + "\"capability\":\"host.null\",\"params\":{}}");

        HostReply reply = adapter.dispatch(req);
        assertTrue(reply.isError());
        assertEquals("INTERNAL", ((HostReply.Error) reply).code());
    }

    private static String fixtureComplete() {
        return readResource("complete.json");
    }

    private static String fixtureError() {
        return readResource("error.json");
    }

    private static String readResource(String name) {
        try (java.io.InputStream in = HostAdapterTest.class
                .getResourceAsStream("/conformance/host/" + name)) {
            if (in == null) {
                throw new AssertionError("missing fixture: " + name);
            }
            return new String(in.readAllBytes(), java.nio.charset.StandardCharsets.UTF_8);
        } catch (java.io.IOException e) {
            throw new AssertionError("failed reading fixture " + name, e);
        }
    }
}
