package com.reader.core.samples

import com.reader.core.ReaderCoreRuntime
import java.util.concurrent.TimeUnit

fun pingReaderCore(): String? {
    ReaderCoreRuntime("{}").use { runtime ->
        runtime.sendCommand("runtime.ping", 1L, "{}")
        return runtime.pollEvent(1, TimeUnit.SECONDS)
    }
}

fun completeHostRequest(): String? {
    ReaderCoreRuntime("{}").use { runtime ->
        runtime.sendCommand(
            "runtime.hostSmoke",
            10L,
            """{"capability":"host.smoke.echo","params":{"message":"android"}}"""
        )

        val hostRequest = runtime.pollEvent(1, TimeUnit.SECONDS)
            ?: error("host.request timed out")

        val operationId = operationIdFrom(hostRequest)
        runtime.sendHostComplete(
            operationId,
            """{"status":"ok","source":"android"}""",
            11L
        )
        return runtime.pollEvent(1, TimeUnit.SECONDS)
    }
}

private fun operationIdFrom(eventJson: String): Long {
    val match = Regex(""""operationId"\s*:\s*(\d+)""").find(eventJson)
        ?: error("host.request missing operationId: $eventJson")
    return match.groupValues[1].toLong()
}
