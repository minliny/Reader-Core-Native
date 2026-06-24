package com.reader.core

/**
 * Native entry point into Reader-Core. Loads `libreader_core_jni.so` once
 * per process; all externals are declared here.
 *
 * Lifecycle:
 *   val handle = NativeCoreBridge.runtimeCreate("{}", listener)
 *   NativeCoreBridge.runtimeSend(handle, commandJson)
 *   ...
 *   NativeCoreBridge.runtimeDestroy(handle)
 *
 * The [handle] is an opaque pointer cast to a Long; do not interpret it.
 * After [runtimeDestroy] the handle is invalid and must not be reused.
 */
object NativeCoreBridge {
    init {
        System.loadLibrary("reader_core_jni")
    }

    /** C ABI version (compile/load-time check). */
    @JvmStatic external fun abiVersion(): Int

    /**
     * One-shot smoke: create a runtime, send `runtime.ping`, capture the first
     * event, destroy, and return its JSON. Not a business API; retained for
     * the build-gate smoke contract.
     */
    @JvmStatic external fun pingSmoke(): String

    /**
     * Create a runtime. [configJson] is the platform config (data/cache
     * directories, etc.); pass `"{}"` for defaults. [listener] receives all
     * Core events for the lifetime of the returned handle.
     *
     * @return an opaque runtime handle. On failure a RuntimeException is thrown.
     */
    @JvmStatic external fun runtimeCreate(configJson: String, listener: ReaderEventListener): Long

    /**
     * Send a JSON command (platform → Core), e.g. `runtime.ping`, `core.info`,
     * `host.complete`, `host.error`.
     *
     * @return 0 on success; non-zero error code otherwise. (Structured error
     *   messages are not yet exposed by the C ABI on the Android baseline —
     *   see bindings/android/STATUS.md. Async failures arrive as `error`
     *   events on the listener, which carry a structured `{code,message,
     *   retryable}` payload.)
     */
    @JvmStatic external fun runtimeSend(handle: Long, commandJson: String): Int

    /** Cancel a pending request by its requestId. Returns 0 (incl. not-found). */
    @JvmStatic external fun runtimeCancel(handle: Long, requestId: Long): Int

    /** Destroy the runtime and release the handle. No further events fire. */
    @JvmStatic external fun runtimeDestroy(handle: Long)
}
