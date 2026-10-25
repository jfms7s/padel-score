package io.github.jfms7s.padelscore.net

import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.JsonNull
import mockwebserver3.MockResponse
import mockwebserver3.MockWebServer
import okhttp3.OkHttpClient
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import java.time.Duration

class PadelApiTest {
    private lateinit var server: MockWebServer
    private var token: String? = "pdl_TEST"

    @Before fun start() {
        server = MockWebServer().apply { start() }
    }

    @After fun stop() = server.close()

    private fun resource(path: String): String =
        checkNotNull(javaClass.classLoader?.getResource(path)) { "missing test resource $path" }.readText()

    private fun api(http: OkHttpClient = OkHttpClient()) =
        HttpPadelApi(server.url("/"), http, WireJson) { token }

    private fun enqueue(code: Int, body: String = "", vararg headers: Pair<String, String>) {
        server.enqueue(MockResponse.Builder().code(code).body(body).apply { headers.forEach { (k, v) -> addHeader(k, v) } }.build())
    }

    private val request = CommandRequest("c1", false, 9, listOf(Command.Point(Team.A)))

    @Test fun send_posts_bearer_and_body() = runBlocking {
        enqueue(200, resource("view-one-point.json"))
        api().send("m1", request)
        val recorded = server.takeRequest()
        assertEquals("POST", recorded.method)
        assertEquals("/api/me/matches/m1/commands", recorded.url.encodedPath)
        assertEquals("Bearer pdl_TEST", recorded.headers["Authorization"])
        assertEquals(WireJson.encodeToString(CommandRequest.serializer(), request), recorded.body?.utf8())
    }

    @Test fun send_maps_each_status() = runBlocking {
        enqueue(200, resource("view-one-point.json"))
        assertTrue(api().send("m1", request) is SendResult.Applied)
        enqueue(409, resource("commands-409-conflict.json"))
        assertTrue(api().send("m1", request) is SendResult.Conflict)
        enqueue(422, resource("commands-422-rejected.json"))
        assertTrue(api().send("m1", request) is SendResult.Rejected)
        enqueue(401, resource("auth-401-unauthorized.json"))
        assertEquals(SendResult.Unauthorized, api().send("m1", request))
        enqueue(404, """{"error":"not_found"}""")
        assertEquals(SendResult.NotFound, api().send("m1", request))
        enqueue(403)
        assertEquals(SendResult.NotFound, api().send("m1", request))
        enqueue(500)
        assertTrue(api().send("m1", request) is SendResult.NotSent)
        enqueue(400, resource("commands-400-invalid.json"))
        assertTrue(api().send("m1", request) is SendResult.NotSent)
        enqueue(200, "not json")
        assertTrue(api().send("m1", request) is SendResult.NotSent)
    }

    @Test fun rejected_without_kind_falls_back_to_json_null() = runBlocking {
        enqueue(422, """{"error":"rejected"}""")
        assertEquals(SendResult.Rejected(JsonNull), api().send("m1", request))
    }

    @Test fun send_timeout_is_not_sent_and_not_retried() = runBlocking {
        server.enqueue(
            MockResponse.Builder().code(200).body(resource("view-one-point.json"))
                .bodyDelay(3, java.util.concurrent.TimeUnit.SECONDS).build(),
        )
        val fast = OkHttpClient.Builder().callTimeout(Duration.ofSeconds(1)).retryOnConnectionFailure(false).build()
        assertTrue(api(fast).send("m1", request) is SendResult.NotSent)
        assertEquals(1, server.requestCount)
    }

    @Test fun send_without_token_is_unauthorized_and_sends_nothing() = runBlocking {
        token = null
        assertEquals(SendResult.Unauthorized, api().send("m1", request))
        assertEquals(0, server.requestCount)
    }

    @Test fun poll_maps_rfc8628_codes() = runBlocking {
        enqueue(200, resource("fixtures/device-token-200.json"))
        assertTrue(api().pollToken("dc") is TokenPoll.Granted)
        for ((name, expected) in listOf(
            "pending" to TokenPoll.Pending,
            "slow-down" to TokenPoll.SlowDown,
            "expired" to TokenPoll.Expired,
            "denied" to TokenPoll.Denied,
        )) {
            enqueue(400, resource("fixtures/device-token-$name.json"))
            assertEquals(name, expected, api().pollToken("dc"))
        }
    }

    @Test fun poll_posts_device_code_as_json() = runBlocking {
        enqueue(400, resource("fixtures/device-token-pending.json"))
        api().pollToken("dc-1")
        assertEquals("""{"device_code":"dc-1"}""", server.takeRequest().body?.utf8())
    }

    @Test fun poll_other_4xx_is_expired_and_5xx_is_failed() = runBlocking {
        enqueue(404)
        assertEquals(TokenPoll.Expired, api().pollToken("dc"))
        enqueue(503)
        assertTrue(api().pollToken("dc") is TokenPoll.Failed)
    }

    @Test fun poll_429_is_too_many_with_retry_after() = runBlocking {
        enqueue(429, resource("fixtures/device-token-429.json"), "Retry-After" to "30")
        assertEquals(TokenPoll.TooMany(30), api().pollToken("dc"))
        enqueue(429, resource("fixtures/device-token-429.json"))
        assertEquals(TokenPoll.TooMany(57), api().pollToken("dc"))
    }

    @Test fun start_posts_trimmed_name_and_decodes() = runBlocking {
        enqueue(200, resource("fixtures/device-start.json"))
        val result = api().startDevice("x".repeat(50))
        assertTrue(result is ApiResult.Ok)
        assertEquals("""{"device_name":"${"x".repeat(40)}"}""", server.takeRequest().body?.utf8())
    }

    @Test fun start_429_reports_retry_after() = runBlocking {
        enqueue(429, "", "Retry-After" to "30")
        assertEquals(ApiResult.TooMany(30), api().startDevice("Watch"))
    }

    @Test fun my_matches_decodes_and_sends_bearer() = runBlocking {
        enqueue(200, resource("fixtures/me-matches.json"))
        val result = api().myMatches()
        assertEquals(1, (result as ApiResult.Ok).value.size)
        assertEquals("Bearer pdl_TEST", server.takeRequest().headers["Authorization"])
    }

    @Test fun my_matches_401_is_unauthorized() = runBlocking {
        enqueue(401, resource("fixtures/me-unauthorized-401.json"))
        assertEquals(ApiResult.Unauthorized, api().myMatches())
    }

    @Test fun stream_requests_carry_bearer_and_path() = runBlocking {
        val list = api().listStreamRequest()
        assertEquals("/api/me/matches/stream", list.url.encodedPath)
        assertEquals("Bearer pdl_TEST", list.header("Authorization"))
        assertEquals("/api/me/matches/m1/stream", api().matchStreamRequest("m1").url.encodedPath)
    }

    @Test fun token_is_read_off_the_calling_thread() = runBlocking {
        val caller = Thread.currentThread()
        val readers = mutableListOf<Thread>()
        val api = HttpPadelApi(server.url("/"), OkHttpClient(), WireJson) {
            synchronized(readers) { readers += Thread.currentThread() }
            "pdl_TEST"
        }
        enqueue(200, resource("fixtures/me-matches.json"))
        api.myMatches()
        enqueue(200, resource("view-one-point.json"))
        api.send("m1", request)
        api.listStreamRequest()
        api.matchStreamRequest("m1")
        assertEquals(4, readers.size)
        assertTrue(readers.none { it === caller })
    }

    @Test fun granted_to_string_hides_the_token() {
        val text = TokenPoll.Granted("pdl_TEST_secret").toString()
        assertFalse(text.contains("pdl_TEST_secret"))
        assertEquals("Granted(token=██)", text)
    }
}
