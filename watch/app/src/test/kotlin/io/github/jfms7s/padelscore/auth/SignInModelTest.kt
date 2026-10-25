package io.github.jfms7s.padelscore.auth

import io.github.jfms7s.padelscore.net.ApiResult
import io.github.jfms7s.padelscore.net.CommandRequest
import io.github.jfms7s.padelscore.net.DeviceStart
import io.github.jfms7s.padelscore.net.MyMatch
import io.github.jfms7s.padelscore.net.PadelApi
import io.github.jfms7s.padelscore.net.SendResult
import io.github.jfms7s.padelscore.net.TokenPoll
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestCoroutineScheduler
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.setMain
import okhttp3.HttpUrl.Companion.toHttpUrl
import okhttp3.Request
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class FakeSignInApi(
    private val start: ApiResult<DeviceStart>,
    private val polls: MutableList<TokenPoll>,
    private val scheduler: TestCoroutineScheduler,
) : PadelApi {
    val pollTimes = mutableListOf<Long>()
    var starts = 0

    override suspend fun startDevice(deviceName: String): ApiResult<DeviceStart> {
        starts++
        return start
    }

    override suspend fun pollToken(deviceCode: String): TokenPoll {
        pollTimes += scheduler.currentTime / 1000
        return if (polls.size > 1) polls.removeAt(0) else polls.first()
    }

    override suspend fun myMatches(): ApiResult<List<MyMatch>> = error("unused")
    override suspend fun send(matchId: String, request: CommandRequest): SendResult = error("unused")
    override suspend fun listStreamRequest(): Request = error("unused")
    override suspend fun matchStreamRequest(matchId: String): Request = error("unused")
}

class MemoryTokenStore(private val failWrite: Boolean = false) : TokenStore {
    var token: String? = null
    override suspend fun read() = token
    override suspend fun write(token: String) {
        if (failWrite) throw IllegalStateException("secret-material")
        this.token = token
    }
    override suspend fun clear() { token = null }
    override val signedIn: Flow<Boolean> = flowOf(false)
}

fun deviceStart(expiresIn: Int = 600, interval: Int = 5, uri: String = "/link") =
    ApiResult.Ok(DeviceStart("dev", "ABCD-2345", uri, expiresIn, interval))

@OptIn(ExperimentalCoroutinesApi::class)
class SignInModelTest {
    private val scheduler = TestCoroutineScheduler()
    private val server = "https://example.test".toHttpUrl()

    @Before fun setMain() = Dispatchers.setMain(StandardTestDispatcher(scheduler))

    @After fun resetMain() = Dispatchers.resetMain()

    private fun model(
        polls: List<TokenPoll>,
        start: ApiResult<DeviceStart> = deviceStart(),
        tokens: TokenStore = MemoryTokenStore(),
    ): Pair<SignInModel, FakeSignInApi> {
        val api = FakeSignInApi(start, polls.toMutableList(), scheduler)
        return SignInModel(api, tokens, server, "SM-R960") to api
    }

    private fun advance(seconds: Long) {
        scheduler.advanceTimeBy(seconds * 1000)
        scheduler.runCurrent()
    }

    @Test
    fun polls_at_interval_until_granted() {
        val tokens = MemoryTokenStore()
        val (model, api) = model(listOf(TokenPoll.Pending, TokenPoll.Pending, TokenPoll.Granted("pdl_TEST")), tokens = tokens)
        advance(20)
        assertEquals(listOf(5L, 10L, 15L), api.pollTimes)
        assertEquals("pdl_TEST", tokens.token)
        assertEquals(SignInState.SignedIn, model.state.value)
    }

    @Test
    fun slow_down_adds_five_seconds() {
        val (_, api) = model(listOf(TokenPoll.SlowDown, TokenPoll.Pending))
        advance(30)
        assertEquals(listOf(5L, 15L, 25L), api.pollTimes.take(3))
    }

    @Test
    fun expires_locally_after_expires_in() {
        val (model, api) = model(listOf(TokenPoll.Pending), deviceStart(expiresIn = 600))
        advance(599)
        assertTrue(model.state.value is SignInState.ShowCode)
        advance(1)
        assertEquals(SignInState.Expired, model.state.value)
        val polls = api.pollTimes.size
        advance(60)
        assertEquals(polls, api.pollTimes.size)
        assertTrue(api.pollTimes.all { it < 600 })
    }

    @Test
    fun denied_stops_polling() {
        val (model, api) = model(listOf(TokenPoll.Denied))
        advance(60)
        assertEquals(SignInState.Denied, model.state.value)
        assertEquals(1, api.pollTimes.size)
    }

    @Test
    fun network_failure_keeps_code_and_polling() {
        val (model, api) = model(listOf(TokenPoll.Failed("network error"), TokenPoll.Pending))
        advance(6)
        assertEquals(true, (model.state.value as SignInState.ShowCode).offline)
        assertEquals("ABCD-2345", (model.state.value as SignInState.ShowCode).userCode)
        advance(5)
        assertEquals(listOf(5L, 10L), api.pollTimes)
        assertFalse((model.state.value as SignInState.ShowCode).offline)
    }

    @Test
    fun too_many_waits_retry_after_and_keeps_polling() {
        val (model, api) = model(listOf(TokenPoll.TooMany(30), TokenPoll.Pending))
        advance(34)
        assertEquals(listOf(5L), api.pollTimes)
        assertTrue(model.state.value is SignInState.ShowCode)
        advance(1)
        assertEquals(listOf(5L, 35L), api.pollTimes)
        advance(5)
        assertEquals(listOf(5L, 35L, 40L), api.pollTimes)
    }

    @Test
    fun too_many_without_retry_after_keeps_interval() {
        val (_, api) = model(listOf(TokenPoll.TooMany(null), TokenPoll.Pending))
        advance(11)
        assertEquals(listOf(5L, 10L), api.pollTimes)
    }

    @Test
    fun minutes_left_counts_down_while_showing_the_code() {
        val (model, _) = model(listOf(TokenPoll.Pending))
        advance(1)
        assertEquals(599, (model.state.value as SignInState.ShowCode).secondsLeft)
        advance(60)
        assertEquals(539, (model.state.value as SignInState.ShowCode).secondsLeft)
    }

    @Test
    fun link_url_is_resolved_against_server() {
        val (model, _) = model(listOf(TokenPoll.Pending))
        advance(1)
        assertEquals("example.test/link", (model.state.value as SignInState.ShowCode).linkUrl)
    }

    @Test
    fun too_many_on_start_fails_and_restart_starts_over() {
        val (model, api) = model(listOf(TokenPoll.Pending), start = ApiResult.TooMany(60))
        advance(1)
        assertEquals(SignInState.Failed(FailKind.TooManyTries), model.state.value)
        model.restart()
        advance(1)
        assertEquals(2, api.starts)
    }

    @Test
    fun unreachable_on_start_fails() {
        val (model, _) = model(listOf(TokenPoll.Pending), start = ApiResult.Failed("network error"))
        advance(1)
        assertEquals(SignInState.Failed(FailKind.Unreachable), model.state.value)
    }

    @Test
    fun restart_after_expiry_gets_a_fresh_code() {
        val (model, api) = model(listOf(TokenPoll.Expired))
        advance(6)
        assertEquals(SignInState.Expired, model.state.value)
        model.restart()
        advance(1)
        assertEquals(2, api.starts)
        assertTrue(model.state.value is SignInState.ShowCode)
    }

    @Test
    fun token_write_failure_fails_without_exception_text() {
        val (model, _) = model(listOf(TokenPoll.Granted("pdl_TEST")), tokens = MemoryTokenStore(failWrite = true))
        advance(6)
        assertEquals(SignInState.Failed(FailKind.CouldNotSave), model.state.value)
        assertFalse(model.state.value.toString().contains("secret-material"))
    }

    @Test
    fun device_name_is_trimmed_and_never_blank() {
        assertEquals("SM-R960", deviceNameOf("  SM-R960 "))
        assertEquals(40, deviceNameOf("x".repeat(60)).length)
        assertEquals("Wear OS watch", deviceNameOf("   "))
        assertEquals("Wear OS watch", deviceNameOf(null))
    }
}
