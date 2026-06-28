package com.reader.core;

public final class ReaderCoreLastError {
    private final int code;
    private final String message;
    public ReaderCoreLastError(int code, String message) { this.code = code; this.message = message; }
    public int code() { return code; }
    public String message() { return message; }
    public boolean isPresent() { return code != 0; }
    @Override public String toString() { return "ReaderCoreLastError{code=" + code + ", message='" + message + "'}"; }
}
