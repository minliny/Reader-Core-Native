package com.reader.core.host;

import org.junit.jupiter.api.Test;

import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Deque;
import java.util.List;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

/**
 * Contract tests for {@link CredentialResolveHandler}, filling the Gap D
 * capability ({@code credential.resolve}) documented in
 * {@code docs/host-app-contracts/02-local-storage-sync.md} §3.4.
 *
 * <p>The shape is a draft protocol contract (not yet in upstream conformance
 * fixtures), so these tests assert the handler's own draft contract: request
 * {@code {credentialHandle}} → result {@code {username, password}}, with
 * provider failure → retryable INTERNAL and unknown handle → non-retryable
 * INTERNAL. The handler is driven through {@link HostAdapter} and end-to-end
 * via {@link HostEventLoop}.
 */
class CredentialResolveHandlerTest {

    @Test
    void resolvesHandleAndCompletesWithUsernamePassword() {
        FakeProvider provider = new FakeProvider();
        provider.put("webdav-default", new Credential("alice", "s3cret"));

        HostAdapter adapter = new HostAdapter();
        adapter.register(CredentialResolveHandler.CAPABILITY,
                new CredentialResolveHandler(provider));

        HostReply reply = adapter.dispatch(request("{\"credentialHandle\":\"webdav-default\"}"));
        assertTrue(reply.isComplete());
        assertEquals("{\"username\":\"alice\",\"password\":\"s3cret\"}",
                ((HostReply.Complete) reply).resultJson());
        assertEquals("webdav-default", provider.lastHandle);
    }

    @Test
    void unknownHandleYieldsNonRetryableInternalError() {
        FakeProvider provider = new FakeProvider(); // empty store

        HostAdapter adapter = new HostAdapter();
        adapter.register(CredentialResolveHandler.CAPABILITY,
                new CredentialResolveHandler(provider));

        HostReply reply = adapter.dispatch(request("{\"credentialHandle\":\"missing\"}"));
        assertTrue(reply.isError());
        HostReply.Error err = (HostReply.Error) reply;
        assertEquals("INTERNAL", err.code());
        assertFalse(err.retryable());
    }

    @Test
    void providerThrowingYieldsRetryableInternalError() {
        FakeProvider provider = new FakeProvider(new java.io.IOException("keystore locked"));

        HostAdapter adapter = new HostAdapter();
        adapter.register(CredentialResolveHandler.CAPABILITY,
                new CredentialResolveHandler(provider));

        HostReply reply = adapter.dispatch(request("{\"credentialHandle\":\"webdav-default\"}"));
        assertTrue(reply.isError());
        HostReply.Error err = (HostReply.Error) reply;
        assertEquals("INTERNAL", err.code());
        assertTrue(err.retryable());
    }

    @Test
    void missingHandleYieldsNonRetryableInternalError() {
        FakeProvider provider = new FakeProvider();
        HostAdapter adapter = new HostAdapter();
        adapter.register(CredentialResolveHandler.CAPABILITY,
                new CredentialResolveHandler(provider));

        HostReply reply = adapter.dispatch(request("{}"));
        assertTrue(reply.isError());
        assertFalse(((HostReply.Error) reply).retryable());
    }

    @Test
    void endToEndViaHostEventLoopSendsCompleteCommand() {
        FakeProvider provider = new FakeProvider();
        provider.put("webdav-default", new Credential("alice", "s3cret"));

        HostAdapter adapter = new HostAdapter();
        adapter.register(CredentialResolveHandler.CAPABILITY,
                new CredentialResolveHandler(provider));

        FakeTransport transport = new FakeTransport();
        transport.enqueue("{\"protocolVersion\":1,\"requestId\":301,\"type\":\"host.request\","
                + "\"operationId\":7,\"capability\":\"credential.resolve\","
                + "\"params\":{\"credentialHandle\":\"webdav-default\"}}");

        HostEventLoop loop = new HostEventLoop(transport, adapter);
        assertTrue(loop.tick());

        assertEquals(1, transport.sent.size());
        String sent = transport.sent.get(0);
        assertTrue(sent.contains("\"method\":\"host.complete\""));
        assertTrue(sent.contains("\"operationId\":7"));
        assertTrue(sent.contains("\"username\":\"alice\""));
        assertTrue(sent.contains("\"password\":\"s3cret\""));
    }

    // --- helpers ----------------------------------------------------------------

    private static HostRequest request(String paramsJson) {
        return HostRequest.parse("{\"protocolVersion\":1,\"requestId\":301,"
                + "\"type\":\"host.request\",\"operationId\":1,"
                + "\"capability\":\"credential.resolve\",\"params\":" + paramsJson + "}");
    }

    private static final class FakeProvider implements CredentialProvider {
        final java.util.Map<String, Credential> store = new java.util.HashMap<>();
        final Exception failure;
        String lastHandle;

        FakeProvider() {
            this(null);
        }

        FakeProvider(Exception failure) {
            this.failure = failure;
        }

        void put(String handle, Credential cred) {
            store.put(handle, cred);
        }

        @Override
        public Credential resolve(String credentialHandle) throws Exception {
            this.lastHandle = credentialHandle;
            if (failure != null) {
                throw failure;
            }
            return store.get(credentialHandle);
        }
    }

    private static final class FakeTransport implements HostTransport {
        final Deque<String> events = new ArrayDeque<>();
        final List<String> sent = new ArrayList<>();

        void enqueue(String e) {
            events.addLast(e);
        }

        @Override
        public String pollEventJson(long timeoutMillis) {
            return events.pollFirst();
        }

        @Override
        public void sendCommand(String commandJson) {
            sent.add(commandJson);
        }
    }
}
