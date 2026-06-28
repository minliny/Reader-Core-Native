package com.reader.core;

public final class NativeCoreBridge {
    static {
        System.loadLibrary("reader_core_jni");
    }

    private NativeCoreBridge() {
    }

    public static int abiVersion() {
        return nativeAbiVersion();
    }

    public static ReaderCoreLastError lastError() {
        return nativeLastError();
    }

    public static native String pingSmoke();

    public static native long runtimeCreate(String configJson, Object listener);

    public static native int runtimeSend(long handle, String commandJson);

    public static native int runtimeCancel(long handle, long requestId);

    public static native void runtimeDestroy(long handle);

    static long create(byte[] configJson) {
        return nativeCreate(configJson);
    }

    static void destroy(long handle) {
        nativeDestroy(handle);
    }

    static int send(long handle, byte[] commandJson) {
        return nativeSend(handle, commandJson);
    }

    static int cancel(long handle, long requestId) {
        return nativeCancel(handle, requestId);
    }

    static byte[] pollEvent(long handle, long timeoutMillis) {
        return nativePollEvent(handle, timeoutMillis);
    }

    private static native int nativeAbiVersion();

    private static native long nativeCreate(byte[] configJson);

    private static native void nativeDestroy(long handle);

    private static native int nativeSend(long handle, byte[] commandJson);

    private static native int nativeCancel(long handle, long requestId);

    private static native byte[] nativePollEvent(long handle, long timeoutMillis);

    private static native ReaderCoreLastError nativeLastError();
}
