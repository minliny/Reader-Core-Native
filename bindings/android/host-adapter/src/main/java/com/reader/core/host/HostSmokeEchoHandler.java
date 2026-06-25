package com.reader.core.host;

/**
 * {@link CapabilityHandler} for the {@code host.smoke.echo} conformance
 * capability. Core's {@code runtime.hostSmoke} command (see
 * {@code protocol/fixtures/conformance/host/request.json}) asks the host to
 * echo back the request params as the completion result, exercising the full
 * {@code host.request} → {@code host.complete} bus without any real platform
 * side effect. This is the smoke capability used by protocol conformance.
 */
public final class HostSmokeEchoHandler implements CapabilityHandler {

    public static final String CAPABILITY = "host.smoke.echo";

    @Override
    public HostReply handle(HostRequest request) {
        // Echo the request params object back as the completion result.
        return HostReply.complete(request.paramsJson());
    }
}
