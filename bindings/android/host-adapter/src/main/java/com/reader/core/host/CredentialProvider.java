package com.reader.core.host;

/**
 * Host-owned credential store mechanism (Keychain / Keystore / encrypted
 * SharedPreferences). The adapter calls this to resolve a credential handle
 * into plaintext for a single {@code credential.resolve} request; the platform
 * supplies the real implementation. Keeping it behind an interface lets
 * {@link CredentialResolveHandler} be unit-tested with a fake and keeps the
 * secret store firmly on the host side — the adapter never reads the keystore
 * directly.
 *
 * <p>Shape follows the draft contract in
 * {@code docs/host-app-contracts/02-local-storage-sync.md} §3.4 (Gap D):
 * <pre>
 *   request:  { "credentialHandle": "webdav-default" }
 *   result:   { "username": "...", "password": "..." }   // or host.error
 * </pre>
 * Credentials live only for the single request lifetime on the Core side; the
 * host never logs them.
 */
public interface CredentialProvider {

    /**
     * Resolve a credential handle, returning the credential or {@code null} if
     * the handle is unknown / the user denied access (the handler turns this
     * into a {@code host.error}).
     */
    Credential resolve(String credentialHandle) throws Exception;
}
