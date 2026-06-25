package com.reader.core.sample

import com.reader.core.NativeCoreBridge
import com.reader.core.ReaderEventListener
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/**
 * Minimal end-to-end sample for the Android JNI SDK.
 *
 * Flow:
 *  1. Create a runtime with a listener.
 *  2. Send `runtime.ping` and await its `result` event.
 *  3. Demonstrate the host bridge: when a `host.request` event arrives,
 *     answer it with a `host.complete` command sent back through runtimeSend.
 *  4. Destroy the runtime.
 *
 * Illustrative only; in a real Android app the listener would route events
 * to whatever consumer needs them (UI, viewmodel, etc.).
 */
object ReaderCoreSample {

    fun run(): String {
        // Holder so the listener — created before the handle exists — can
        // reference the handle once runtimeCreate returns. Core only emits
        // events after create succeeds, so the assignment always wins the race.
        val handleHolder = LongArray(1)
        val pingResult = CountDownLatch(1)
        var pingEventJson: String? = null

        val listener = object : ReaderEventListener {
            override fun onEvent(eventJson: String) {
                when {
                    eventJson.contains("\"type\":\"result\"") -> {
                        pingEventJson = eventJson
                        pingResult.countDown()
                    }
                    eventJson.contains("\"type\":\"host.request\"") -> {
                        answerHostRequest(handleHolder[0], eventJson)
                    }
                    eventJson.contains("\"type\":\"error\"") -> {
                        pingEventJson = eventJson
                        pingResult.countDown()
                    }
                }
            }
        }

        val handle = NativeCoreBridge.runtimeCreate("{}", listener)
        handleHolder[0] = handle

        val pingCommand =
            """{"protocolVersion":1,"requestId":1,"method":"runtime.ping","params":{}}"""
        require(NativeCoreBridge.runtimeSend(handle, pingCommand) == 0) {
            "runtime.ping send failed (non-zero status)"
        }

        if (!pingResult.await(2, TimeUnit.SECONDS)) {
            NativeCoreBridge.runtimeDestroy(handle)
            error("timed out waiting for runtime.ping result")
        }

        NativeCoreBridge.runtimeDestroy(handle)
        return pingEventJson ?: error("no ping event captured")
    }

    /**
     * Answer a `host.request` event with a `host.complete` command. A real app
     * would dispatch on `capability`/`params`; this sample echoes a minimal ok.
     */
    private fun answerHostRequest(handle: Long, eventJson: String) {
        val operationId = extractLong(eventJson, "\"operationId\":") ?: return
        val complete =
            """{"protocolVersion":1,"requestId":$operationId,"method":"host.complete",""" +
                """"params":{"operationId":$operationId,"result":{"status":"ok","source":"sample"}}}"""
        NativeCoreBridge.runtimeSend(handle, complete)
    }

    private fun extractLong(json: String, key: String): Long? {
        val start = json.indexOf(key)
        if (start < 0) return null
        val sb = StringBuilder()
        var j = start + key.length
        while (j < json.length && (json[j].isDigit() || json[j] == '-')) {
            sb.append(json[j]); j++
        }
        return sb.toString().toLongOrNull()
    }
}
