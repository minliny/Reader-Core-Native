package com.reader.core.host;

/**
 * Outcome of a host-initiated command: either a matching {@code result} event
 * (success), a matching {@code error} event (error), or a timeout. The
 * {@code requestId} correlates to the command {@link HostCommander} sent.
 */
public abstract class CommandResult {

    private final long requestId;
    private final Kind kind;

    private CommandResult(long requestId, Kind kind) {
        this.requestId = requestId;
        this.kind = kind;
    }

    public long requestId() {
        return requestId;
    }

    public Kind kind() {
        return kind;
    }

    public boolean isSuccess() {
        return kind == Kind.SUCCESS;
    }

    public boolean isError() {
        return kind == Kind.ERROR;
    }

    public boolean isTimeout() {
        return kind == Kind.TIMEOUT;
    }

    /** The {@code result} event's {@code data} object JSON. Throws if not success. */
    public String dataJson() {
        throw new IllegalStateException("not a success: " + kind);
    }

    /** The {@code error} event's {@code error} object JSON. Throws if not an error. */
    public String errorJson() {
        throw new IllegalStateException("not an error: " + kind);
    }

    static CommandResult success(long requestId, String dataJson) {
        return new Success(requestId, dataJson);
    }

    static CommandResult error(long requestId, String errorJson) {
        return new Error(requestId, errorJson);
    }

    static CommandResult timeout(long requestId) {
        return new Timeout(requestId);
    }

    public enum Kind {
        SUCCESS, ERROR, TIMEOUT
    }

    private static final class Success extends CommandResult {
        private final String dataJson;

        Success(long requestId, String dataJson) {
            super(requestId, Kind.SUCCESS);
            this.dataJson = dataJson;
        }

        @Override
        public String dataJson() {
            return dataJson;
        }
    }

    private static final class Error extends CommandResult {
        private final String errorJson;

        Error(long requestId, String errorJson) {
            super(requestId, Kind.ERROR);
            this.errorJson = errorJson;
        }

        @Override
        public String errorJson() {
            return errorJson;
        }
    }

    private static final class Timeout extends CommandResult {
        Timeout(long requestId) {
            super(requestId, Kind.TIMEOUT);
        }
    }
}
