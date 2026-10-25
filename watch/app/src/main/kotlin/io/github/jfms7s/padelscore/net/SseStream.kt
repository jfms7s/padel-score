package io.github.jfms7s.padelscore.net

import android.util.Log
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.buffer
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.flow.flow
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.sse.EventSource
import okhttp3.sse.EventSourceListener
import okhttp3.sse.EventSources
import kotlin.time.Duration
import kotlin.time.Duration.Companion.seconds

sealed interface StreamItem<out T> {
    data class Frame<T>(val value: T) : StreamItem<T>
    data object Live : StreamItem<Nothing>
    data object Offline : StreamItem<Nothing>
    data object Unauthorized : StreamItem<Nothing>
}

val DEFAULT_BACKOFF: List<Duration> = listOf(1.seconds, 2.seconds, 5.seconds, 10.seconds)

/**
 * Cold flow over one SSE endpoint: collecting connects, cancelling disconnects.
 *
 * Silence detection is the client's read timeout (the server keep-alives every 10 s, which
 * okhttp-sse never surfaces). Any drop, clean end or non-401 HTTP error emits [StreamItem.Offline]
 * once per outage and reconnects after the next [backoff] step (the last step repeats); a successful
 * open resets the steps. A 401 emits [StreamItem.Unauthorized] and completes.
 *
 * [request] is called per connect so each attempt carries the current token.
 */
fun <T> sseFlow(
    client: OkHttpClient,
    request: suspend () -> Request,
    event: String,
    decode: (String) -> T,
    backoff: List<Duration> = DEFAULT_BACKOFF,
    delay: suspend (Duration) -> Unit = { kotlinx.coroutines.delay(it) },
): Flow<StreamItem<T>> {
    require(backoff.isNotEmpty()) { "backoff must have at least one step" }
    val factory = EventSources.createFactory(client)
    return flow {
        var step = 0
        var offline = false
        while (true) {
            var unauthorized = false
            connect(factory, request(), event, decode).collect { signal ->
                when (signal) {
                    Signal.Open -> {
                        step = 0
                        offline = false
                        emit(StreamItem.Live)
                    }
                    is Signal.Data -> emit(StreamItem.Frame(signal.value))
                    is Signal.End -> unauthorized = signal.unauthorized
                }
            }
            if (unauthorized) {
                emit(StreamItem.Unauthorized)
                return@flow
            }
            if (!offline) {
                offline = true
                emit(StreamItem.Offline)
            }
            delay(backoff[step.coerceAtMost(backoff.lastIndex)])
            step++
        }
    }
}

private sealed interface Signal<out T> {
    data object Open : Signal<Nothing>
    data class Data<T>(val value: T) : Signal<T>
    data class End(val unauthorized: Boolean) : Signal<Nothing>
}

/** One connection attempt; completes after its [Signal.End]. Cancelling it cancels the call. */
private fun <T> connect(
    factory: EventSource.Factory,
    request: Request,
    event: String,
    decode: (String) -> T,
): Flow<Signal<T>> = callbackFlow {
    val source = factory.newEventSource(
        request,
        object : EventSourceListener() {
            override fun onOpen(eventSource: EventSource, response: Response) {
                trySend(Signal.Open)
            }

            override fun onEvent(eventSource: EventSource, id: String?, type: String?, data: String) {
                if (type != event) return
                val value = try {
                    decode(data)
                } catch (_: IllegalArgumentException) {
                    // SerializationException is an IllegalArgumentException. Never log the payload.
                    Log.w(TAG, "skipped an undecodable '$event' frame")
                    return
                }
                trySend(Signal.Data(value))
            }

            override fun onClosed(eventSource: EventSource) {
                end(unauthorized = false)
            }

            override fun onFailure(eventSource: EventSource, t: Throwable?, response: Response?) {
                end(unauthorized = response?.code == 401)
            }

            private fun end(unauthorized: Boolean) {
                trySend(Signal.End(unauthorized))
                channel.close()
            }
        },
    )
    awaitClose { source.cancel() }
}.buffer(Channel.UNLIMITED) // the reader thread must never drop Open/End because the collector is busy

private const val TAG = "SseStream"
