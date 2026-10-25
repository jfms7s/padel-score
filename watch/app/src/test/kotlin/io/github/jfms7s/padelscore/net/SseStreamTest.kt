package io.github.jfms7s.padelscore.net

import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.onEach
import kotlinx.coroutines.flow.take
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.currentTime
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.builtins.ListSerializer
import mockwebserver3.MockResponse
import mockwebserver3.MockResponseBody
import mockwebserver3.MockWebServer
import okhttp3.OkHttpClient
import okhttp3.Request
import okio.BufferedSink
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import java.time.Duration
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

// Robolectric only so android.util.Log works on the undecodable-frame path.
@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(AndroidJUnit4::class)
class SseStreamTest {
    private lateinit var server: MockWebServer
    private val stalls = mutableListOf<CountDownLatch>()
    private val client = OkHttpClient.Builder().retryOnConnectionFailure(false).build()

    @Before fun start() {
        server = MockWebServer().apply { start() }
    }

    @After fun stop() {
        stalls.forEach(CountDownLatch::countDown)
        server.close()
    }

    private fun resource(path: String): String =
        checkNotNull(javaClass.classLoader?.getResource(path)) { "missing test resource $path" }.readText()

    private val stateFrame by lazy { resource("stream-state.txt") }
    private val stateView by lazy { decodeView(stateFrame.lines().first { it.startsWith("data: ") }.removePrefix("data: ")) }

    private fun decodeView(data: String): MatchView = WireJson.decodeFromString(MatchView.serializer(), data)

    private fun request(): Request = Request.Builder().url(server.url("/api/me/matches/m1/stream")).build()

    private fun stateFlow(http: OkHttpClient = client, onRequest: () -> Unit = {}): Flow<StreamItem<MatchView>> =
        sseFlow(http, { onRequest(); request() }, "state", ::decodeView)

    private fun stream(body: String) = server.enqueue(
        MockResponse.Builder().addHeader("Content-Type", "text/event-stream").body(body).build(),
    )

    /** Sends [head], then holds the connection open with no bytes until the test ends. */
    private fun stallingStream(head: String) {
        val release = CountDownLatch(1).also { stalls += it }
        val body = object : MockResponseBody {
            override val contentLength = 1_000_000L

            override fun writeTo(sink: BufferedSink) {
                sink.writeUtf8(head).flush()
                release.await(10, TimeUnit.SECONDS)
            }
        }
        server.enqueue(MockResponse.Builder().addHeader("Content-Type", "text/event-stream").body(body).build())
    }

    private fun status(code: Int, body: String = "") = server.enqueue(MockResponse.Builder().code(code).body(body).build())

    @Test fun emits_live_then_frames() = runTest {
        stream(stateFrame)
        val items = stateFlow().take(2).toList()
        assertEquals(listOf(StreamItem.Live, StreamItem.Frame(stateView)), items)
        assertEquals(0L, (items[1] as StreamItem.Frame).value.revision)
        assertEquals("3941bd88-7bf0-45db-9274-03cae64e29d0", stateView.id)
    }

    @Test fun ignores_other_event_types() = runTest {
        stream(resource("fixtures/me-match-stream.txt") + resource("fixtures/me-matches-stream.txt"))
        val matches = ListSerializer(MyMatch.serializer())
        val items = sseFlow(client, { request() }, "matches", { WireJson.decodeFromString(matches, it) })
            .take(3).toList()
        // The state frame came first on the wire but is not this stream's event; then a clean end.
        assertEquals(StreamItem.Live, items[0])
        assertTrue((items[1] as StreamItem.Frame).value.isNotEmpty())
        assertEquals(StreamItem.Offline, items[2])
    }

    @Test fun skips_undecodable_frame() = runTest {
        stream("event: state\ndata: not json\n\n$stateFrame")
        assertEquals(listOf(StreamItem.Live, StreamItem.Frame(stateView)), stateFlow().take(2).toList())
    }

    @Test fun silence_marks_offline_and_reconnects() = runTest {
        stallingStream(stateFrame)
        stream(stateFrame)
        val fast = client.newBuilder().readTimeout(Duration.ofSeconds(1)).build()
        var frameAt = 0L
        var offlineAt = 0L
        val items = stateFlow(fast).onEach {
            if (it is StreamItem.Frame && frameAt == 0L) frameAt = System.nanoTime()
            if (it == StreamItem.Offline) offlineAt = System.nanoTime()
        }.take(4).toList()
        assertEquals(
            listOf(StreamItem.Live, StreamItem.Frame(stateView), StreamItem.Offline, StreamItem.Live),
            items,
        )
        val silentMs = (offlineAt - frameAt) / 1_000_000
        assertTrue("offline after ${silentMs}ms", silentMs in 800..5_000)
        assertEquals(2, server.requestCount)
    }

    @Test fun backoff_steps_and_resets() = runTest {
        repeat(3) { status(503) }
        stream(stateFrame)
        status(503)
        stream(stateFrame)
        val requestTimes = mutableListOf<Long>()
        val items = stateFlow { requestTimes += currentTime }.take(5).toList()
        // Offline once for the whole failing run, not once per failed attempt.
        assertEquals(
            listOf(StreamItem.Offline, StreamItem.Live, StreamItem.Frame(stateView), StreamItem.Offline, StreamItem.Live),
            items,
        )
        // 1 s, 2 s, 5 s between failed attempts; after the open, back to 1 s, then 2 s.
        assertEquals(listOf(0L, 1_000L, 3_000L, 8_000L, 9_000L, 11_000L), requestTimes)
    }

    @Test fun backoff_caps_at_last_step() = runTest {
        repeat(6) { status(503) }
        stream(stateFrame)
        val requestTimes = mutableListOf<Long>()
        stateFlow { requestTimes += currentTime }.first { it == StreamItem.Live }
        assertEquals(listOf(0L, 1_000L, 3_000L, 8_000L, 18_000L, 28_000L, 38_000L), requestTimes)
    }

    @Test fun not_found_goes_offline_and_retries() = runTest {
        status(404, """{"error":"not_found"}""")
        stream(stateFrame)
        assertEquals(listOf(StreamItem.Offline, StreamItem.Live), stateFlow().take(2).toList())
        assertEquals(2, server.requestCount)
    }

    @Test fun unauthorized_completes_without_reconnect() = runTest {
        status(401, resource("fixtures/me-unauthorized-401.json"))
        stream(stateFrame)
        assertEquals(listOf(StreamItem.Unauthorized), stateFlow().toList())
        assertEquals(1, server.requestCount)
    }

    @Test fun builds_a_fresh_request_per_connect() = runTest {
        status(503)
        stream(stateFrame)
        var built = 0
        stateFlow { built++ }.first { it == StreamItem.Live }
        assertEquals(2, built)
    }

    @Test fun cancel_closes_connection() = runTest {
        stallingStream(stateFrame)
        stateFlow().first { it is StreamItem.Frame }
        awaitNoRunningCalls()
    }

    @Test fun cancel_during_backoff_stops_reconnecting() = runTest {
        status(503)
        stream(stateFrame)
        val offline = CompletableDeferred<Unit>()
        val job = launch { stateFlow().collect { if (it == StreamItem.Offline) offline.complete(Unit) } }
        offline.await()
        runCurrent()
        job.cancel()
        runCurrent()
        assertTrue(job.isCompleted)
        advanceTimeBy(60_000)
        assertEquals(1, server.requestCount)
    }

    // OkHttp finishes a cancelled call on its own thread, so give it a moment in real time.
    private fun awaitNoRunningCalls() {
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(5)
        while (client.dispatcher.runningCallsCount() > 0 && System.nanoTime() < deadline) Thread.sleep(10)
        assertEquals(0, client.dispatcher.runningCallsCount())
    }
}
