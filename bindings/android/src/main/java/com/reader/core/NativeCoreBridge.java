package com.reader.core;

final class NativeCoreBridge {
    static {
        System.loadLibrary("reader_core_jni");
    }

    private NativeCoreBridge() {
    }

    static int abiVersion() {
        return nativeAbiVersion();
    }

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
}
