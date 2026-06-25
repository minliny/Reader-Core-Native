package com.reader.core.host;

import java.util.LinkedHashMap;
import java.util.Map;

/**
 * {@link CapabilityHandler} for the {@code credential.resolve} capability,
 * filling the protocol gap documented as Gap D in
 * {@code docs/host-app-contracts/02-local-storage-sync.md} §3.4.
 *
 * <p>Core sends {@code {credentialHandle}}; this handler delegates to the
 * host-owned {@link CredentialProvider} (Keychain / Keystore) and returns
 * {@code host.complete} with {@code {username, password}}, or {@code host.error}
 * when the handle is unknown / denied / the provider fails. The adapter never
 * touches the keystore directly — secret storage stays on the host side.
 *
 * <p>Draft contract (not yet in upstream conformance fixtures — tracked as a
 * protocol gap with owner "protocol schema + 03 login/auth"):
 * <pre>
 *   request:  { "credentialHandle": "webdav-default" }
 *   result:   { "username": "...", "password": "..." }
 * </pre>
 */
public final class CredentialResolveHandler implements CapabilityHandler {

    public static final String CAPABILITY = "credential.resolve";

    private static final String INTERNAL = "INTERNAL";

    private final CredentialProvider provider;

    public CredentialResolveHandler(CredentialProvider provider) {
        if (provider == null) {
            throw new IllegalArgumentException("provider required");
        }
        this.provider = provider;
    }

    @Override
    @SuppressWarnings("unchecked")
    public HostReply handle(HostRequest request) {
        Object parsed;
        try {
            parsed = Json.parse(request.paramsJson());
        } catch (Json.JsonException e) {
            return HostReply.error(INTERNAL,
                    "invalid credential.resolve params: " + e.getMessage(), false);
        }
        if (!(parsed instanceof Map)) {
            return HostReply.error(INTERNAL, "credential.resolve params must be an object", false);
        }
        Object handleVal = ((Map<String, Object>) parsed).get("credentialHandle");
        if (!(handleVal instanceof String) || ((String) handleVal).isEmpty()) {
            return HostReply.error(INTERNAL,
                    "credential.resolve requires non-empty credentialHandle", false);
        }
        String handle = (String) handleVal;

        Credential cred;
        try {
            cred = provider.resolve(handle);
        } catch (Exception e) {
            // Provider failure (keystore locked, I/O, etc.) — transient, retryable.
            return HostReply.error(INTERNAL,
                    "credential.resolve failed: " + e.getMessage(), true);
        }
        if (cred == null) {
            // Unknown handle or user denial — not transient.
            return HostReply.error(INTERNAL,
                    "credential not found for handle: " + handle, false);
        }
        return HostReply.complete(buildResultJson(cred));
    }

    private static String buildResultJson(Credential cred) {
        Map<String, Object> result = new LinkedHashMap<>();
        result.put("username", cred.username());
        result.put("password", cred.password());
        return Json.stringify(result);
    }
}
