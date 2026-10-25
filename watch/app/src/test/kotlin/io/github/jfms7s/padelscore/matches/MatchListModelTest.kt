package io.github.jfms7s.padelscore.matches

import io.github.jfms7s.padelscore.net.ApiResult
import io.github.jfms7s.padelscore.net.CommandRequest
import io.github.jfms7s.padelscore.net.Current
import io.github.jfms7s.padelscore.net.DeviceStart
import io.github.jfms7s.padelscore.net.MatchView
import io.github.jfms7s.padelscore.net.MyMatch
import io.github.jfms7s.padelscore.net.PadelApi
import io.github.jfms7s.padelscore.net.Player
import io.github.jfms7s.padelscore.net.Score
import io.github.jfms7s.padelscore.net.SendResult
import io.github.jfms7s.padelscore.net.Status
import io.github.jfms7s.padelscore.net.StreamItem
import io.github.jfms7s.padelscore.net.Team
import io.github.jfms7s.padelscore.net.Teams
import io.github.jfms7s.padelscore.net.TokenPoll
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestCoroutineScheduler
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.setMain
import okhttp3.Request
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.Test

fun viewOf(id: String, a: List<String>, b: List<String>) = MatchView(
    id = id,
    teams = Teams(a, b),
    revision = 0,
    headSeq = 0,
    events = emptyList(),
    score = Score(listOf(0, 0), listOf(0, 0), Current.None, Player(Team.A, 0), false, Status.InProgress),
)

class FakeListApi(var result: ApiResult<List<MyMatch>>) : PadelApi {
    var gets = 0

    override suspend fun myMatches(): ApiResult<List<MyMatch>> {
        gets++
        return result
    }

    override suspend fun startDevice(deviceName: String): ApiResult<DeviceStart> = error("unused")
    override suspend fun pollToken(deviceCode: String): TokenPoll = error("unused")
    override suspend fun send(matchId: String, request: CommandRequest): SendResult = error("unused")
    override suspend fun listStreamRequest(): Request = error("unused")
    override suspend fun matchStreamRequest(matchId: String): Request = error("unused")
}

@OptIn(ExperimentalCoroutinesApi::class)
class MatchListModelTest {
    private val scheduler = TestCoroutineScheduler()
    private val stream = MutableSharedFlow<StreamItem<List<MyMatch>>>(extraBufferCapacity = 8)
    private val first = MyMatch("m1", "Ana / Rui v João / Marta")
    private val second = MyMatch("m2", null)

    @Before fun setMain() = Dispatchers.setMain(StandardTestDispatcher(scheduler))

    @After fun resetMain() = Dispatchers.resetMain()

    private fun model(result: ApiResult<List<MyMatch>>): Pair<MatchListModel, FakeListApi> {
        val api = FakeListApi(result)
        val model = MatchListModel(api, stream, QUICK)
        scheduler.runCurrent()
        return model to api
    }

    private fun emit(item: StreamItem<List<MyMatch>>) {
        stream.tryEmit(item)
        scheduler.runCurrent()
    }

    private fun ready(model: MatchListModel) = model.state.value as ListState.Ready

    @Test
    fun initial_get_then_stream_replaces_rows() {
        val (model, _) = model(ApiResult.Ok(listOf(first)))
        assertEquals(listOf("m1"), ready(model).rows.map { it.id })
        emit(StreamItem.Frame(listOf(second, first)))
        assertEquals(listOf("m2", "m1"), ready(model).rows.map { it.id })
        emit(StreamItem.Frame(emptyList()))
        assertEquals(emptyList<MatchRow>(), ready(model).rows)
    }

    @Test
    fun offline_keeps_rows_and_marks_not_live() {
        val (model, _) = model(ApiResult.Ok(listOf(first)))
        emit(StreamItem.Frame(listOf(first)))
        assertEquals(true, ready(model).live)
        emit(StreamItem.Offline)
        assertEquals(false, ready(model).live)
        assertEquals(listOf("m1"), ready(model).rows.map { it.id })
        emit(StreamItem.Live)
        assertEquals(true, ready(model).live)
    }

    @Test
    fun unauthorized_from_stream_or_get_is_reported() {
        val (viaGet, _) = model(ApiResult.Unauthorized)
        assertEquals(ListState.Unauthorized, viaGet.state.value)

        val (viaStream, _) = model(ApiResult.Ok(listOf(first)))
        emit(StreamItem.Unauthorized)
        assertEquals(ListState.Unauthorized, viaStream.state.value)
    }

    @Test
    fun row_uses_label_or_quick_match_and_team_names() {
        val labelled = rowOf(MyMatch("m1", "Server label", viewOf("m1", listOf("X"), listOf("Y"))), QUICK)
        assertEquals(MatchRow("m1", QUICK, "Server label"), labelled)

        val fromView = rowOf(MyMatch("m2", null, viewOf("m2", listOf("Ana", "Rui"), listOf("João", "Marta"))), QUICK)
        assertEquals("Ana / Rui v João / Marta", fromView.subtitle)
        assertEquals(QUICK, fromView.title)

        assertEquals(MatchRow("m3", QUICK, ""), rowOf(MyMatch("m3", null, null), QUICK))
    }

    @Test
    fun failed_get_is_keyed_and_a_stream_frame_recovers_it() {
        val (model, _) = model(ApiResult.Failed("io"))
        assertEquals(ListState.Failed(ListFailKind.Unreachable), model.state.value)
        emit(StreamItem.Frame(listOf(first)))
        assertEquals(listOf("m1"), ready(model).rows.map { it.id })

        val (limited, _) = model(ApiResult.TooMany(3))
        assertEquals(ListState.Failed(ListFailKind.TooManyTries), limited.state.value)
    }

    @Test
    fun failed_get_keeps_rows_the_stream_already_filled() {
        val api = FakeListApi(ApiResult.Failed("io"))
        val model = MatchListModel(api, flowOf(StreamItem.Frame(listOf(first))), QUICK)
        scheduler.runCurrent()
        assertEquals(listOf("m1"), ready(model).rows.map { it.id })
    }

    @Test
    fun refresh_reruns_the_get() {
        val (model, api) = model(ApiResult.Failed("io"))
        api.result = ApiResult.Ok(listOf(first))
        model.refresh()
        scheduler.runCurrent()
        assertEquals(2, api.gets)
        assertEquals(listOf("m1"), ready(model).rows.map { it.id })
    }

    private companion object {
        const val QUICK = "Quick match"
    }
}
