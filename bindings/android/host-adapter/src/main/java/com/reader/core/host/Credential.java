package com.reader.core.host;

/** Resolved credential for a single {@code credential.resolve} request. */
public final class Credential {

    private final String username;
    private final String password;

    public Credential(String username, String password) {
        this.username = username;
        this.password = password;
    }

    public String username() {
        return username;
    }

    public String password() {
        return password;
    }
}
