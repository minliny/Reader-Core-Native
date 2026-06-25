package com.reader.core.host;

/**
 * The host's response to a {@link HostRequest}. Either a successful
 * {@code host.complete} carrying a result object, or a {@code host.error}
 * carrying a code/message/retryable triple. Encoded to wire JSON by
 * {@link HostReplyCodec}.
 */
public abstract class HostReply {

    private HostReply() {}

    public abstract String kind();

    public static HostReply complete(String resultJson) {
        return new Complete(resultJson == null ? "{}" : resultJson);
    }

    public static HostReply error(String code, String message, boolean retryable) {
        if (code == null || code.isEmpty()) {
            throw new IllegalArgumentException("error code required");
        }
        return new Error(code, message == null ? "" : message, retryable);
    }

    public boolean isComplete() {
        return this instanceof Complete;
    }

    public boolean isError() {
        return this instanceof Error;
    }

    public static final class Complete extends HostReply {
        private final String resultJson;

        Complete(String resultJson) {
            super();
            this.resultJson = resultJson;
        }

        public String resultJson() {
            return resultJson;
        }

        @Override
        public String kind() {
            return "complete";
        }
    }

    public static final class Error extends HostReply {
        private final String code;
        private final String message;
        private final boolean retryable;

        Error(String code, String message, boolean retryable) {
            super();
            this.code = code;
            this.message = message;
            this.retryable = retryable;
        }

        public String code() {
            return code;
        }

        public String message() {
            return message;
        }

        public boolean retryable() {
            return retryable;
        }

        @Override
        public String kind() {
            return "error";
        }
    }
}
