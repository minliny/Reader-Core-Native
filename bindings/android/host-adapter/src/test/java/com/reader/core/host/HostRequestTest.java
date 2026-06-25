package com.reader.core.host;

import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;

/** Contract tests for {@link HostRequest} parsing against the event schema. */
class HostRequestTest {

    @Test
    void parsesValidHostRequestEvent() {
        String event = "{\"protocolVersion\":1,\"requestId\":301,"
                + "\"type\":\"host.request\",\"operationId\":1,"
                + "\"capability\":\"host.smoke.echo\","
                + "\"params\":{\"message\":\"conformance host request\"}}";
        HostRequest req = HostRequest.parse(event);
        assertEquals(301L, req.requestId());
        assertEquals(1L, req.operationId());
        assertEquals("host.smoke.echo", req.capability());
        assertEquals("{\"message\":\"conformance host request\"}", req.paramsJson());
    }

    @Test
    void rejectsOperationIdZero() {
        String event = "{\"protocolVersion\":1,\"requestId\":1,"
                + "\"type\":\"host.request\",\"operationId\":0,"
                + "\"capability\":\"host.smoke.echo\",\"params\":{}}";
        assertThrows(IllegalArgumentException.class, () -> HostRequest.parse(event));
    }

    @Test
    void rejectsMissingCapability() {
        String event = "{\"protocolVersion\":1,\"requestId\":1,"
                + "\"type\":\"host.request\",\"operationId\":1,\"params\":{}}";
        assertThrows(IllegalArgumentException.class, () -> HostRequest.parse(event));
    }

    @Test
    void rejectsCapabilityWithoutDot() {
        String event = "{\"protocolVersion\":1,\"requestId\":1,"
                + "\"type\":\"host.request\",\"operationId\":1,"
                + "\"capability\":\"noscheme\",\"params\":{}}";
        assertThrows(IllegalArgumentException.class, () -> HostRequest.parse(event));
    }

    @Test
    void rejectsWrongType() {
        String event = "{\"protocolVersion\":1,\"requestId\":1,"
                + "\"type\":\"result\",\"operationId\":1,"
                + "\"capability\":\"host.smoke.echo\",\"params\":{}}";
        assertThrows(IllegalArgumentException.class, () -> HostRequest.parse(event));
    }

    @Test
    void rejectsMalformedJson() {
        assertThrows(IllegalArgumentException.class,
                () -> HostRequest.parse("{not json"));
    }

    @Test
    void rejectsNonObjectRoot() {
        assertThrows(IllegalArgumentException.class,
                () -> HostRequest.parse("[1,2,3]"));
    }
}
