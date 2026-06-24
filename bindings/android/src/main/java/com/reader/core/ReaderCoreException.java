package com.reader.core;

public final class ReaderCoreException extends RuntimeException {
    private final int status;

    public ReaderCoreException(String message) {
        this(-1, message);
    }

    public ReaderCoreException(int status, String message) {
        super(message);
        this.status = status;
    }

    public int status() {
        return status;
    }
}
