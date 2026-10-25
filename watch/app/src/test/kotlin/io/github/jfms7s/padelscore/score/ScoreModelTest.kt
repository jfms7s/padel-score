package io.github.jfms7s.padelscore.score

import io.github.jfms7s.padelscore.haptics.Haptics
import io.github.jfms7s.padelscore.net.ApiResult
import io.github.jfms7s.padelscore.net.Command
import io.github.jfms7s.padelscore.net.CommandRequest
import io.github.jfms7s.padelscore.net.DeviceStart
import io.github.jfms7s.padelscore.net.MatchView
import io.github.jfms7s.padelscore.net.MyMatch
import io.github.jfms7s.padelscore.net.PadelApi
import io.github.jfms7s.padelscore.net.SendResult
import io.github.jfms7s.padelscore.net.StreamItem
import io.github.jfms7s.padelscore.net.Team
import io.github.jfms7s.padelscore.net.TokenPoll
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestCoroutineScheduler
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.setMain
import kotlinx.serialization.json.JsonPrimitive
import okhttp3.Request
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

/** Records every send; with [hold] set, each send waits until the test calls [release]. */
class FakeSendApi : PadelApi {
    val requests = mutableListOf<Pair<String, CommandRequest>>()
    var result: SendResult = SendResult.NotSent("unset")
    var hold = false
    var failure: Exception? = null
    private var gate = CompletableDeferred<Unit>()

    fun release() = gate.complete(Unit)

    override suspend fun send(matchId: String, request: CommandRequest): SendResult {
        requests += matchId to request
        if (hold) gate.await().also { gate = CompletableDeferred() }
        failure?.let { throw it }
        return result
    }

    override suspend fun startDevice(deviceName: String): ApiResult<DeviceStart> = error("unused")
    override suspend fun pollToken(deviceCode: String): TokenPoll = error("unused")
    override suspend fun myMatches(): ApiResult<List<MyMatch>> = error("unused")
    override suspend fun listStreamRequest(): Request = error("unused")
    override suspend fun matchStreamRequest(matchId: String): Request = error("unused")
}

class FakeHaptics : Haptics {
    val calls = mutableListOf<String>()
    override fun accepted() {
        calls += "accepted"
    }
    override fun notSent() {
        calls += "notSent"
    }
    override fun refused() {
        calls += "refused"
    }
}

@OptIn(ExperimentalCoroutinesApi::class)
class ScoreModelTest {
    private val scheduler = TestCoroutineScheduler()
    private val stream = MutableSharedFlow<StreamItem<MatchView>>(extraBufferCapacity = 8)
    private val api = FakeSendApi()
    private val haptics = FakeHaptics()
    private var ids = 0
    private val onePoint = fixtureView("view-one-point.json") // revision 1, events [1]

    @Before fun setMain() = Dispatchers.setMain(StandardTestDispatcher(scheduler))

    @After fun resetMain() = Dispatchers.resetMain()

    /** A started model already showing [shown]. */
    private fun model(shown: MatchView = onePoint): ScoreModel {
        val model = ScoreModel(MATCH, api, stream, haptics, newId = { "id-${++ids}" })
        model.startStream()
        scheduler.runCurrent()
        emit(StreamItem.Live)
        emit(StreamItem.Frame(shown))
        return model
    }

    private fun emit(item: StreamItem<MatchView>) {
        stream.tryEmit(item)
        scheduler.runCurrent()
    }

    private fun tap(model: ScoreModel, team: Team = Team.A) {
        model.point(team)
        scheduler.runCurrent()
    }

    @Test
    fun tap_sends_point_with_current_revision_and_fresh_client_id() {
        val model = model()
        api.result = SendResult.Applied(onePoint.copy(revision = 2))
        tap(model, Team.B)
        tap(model, Team.A)
        assertEquals(
            listOf(
                MATCH to CommandRequest("id-1", false, 1, listOf(Command.Point(Team.B))),
                MATCH to CommandRequest("id-2", false, 2, listOf(Command.Point(Team.A))),
            ),
            api.requests,
        )
    }

    @Test
    fun applied_shows_server_view_and_buzzes_accepted() {
        val model = model()
        val next = fixtureView("view-advantage.json").copy(revision = 2)
        api.result = SendResult.Applied(next)
        tap(model)
        val ui = model.ui.value
        assertSame(next, ui.view)
        assertEquals(Feedback.Accepted, ui.feedback)
        assertFalse(ui.sending)
        assertEquals(listOf("accepted"), haptics.calls)
    }

    @Test
    fun taps_while_sending_are_ignored() {
        val model = model()
        api.hold = true
        api.result = SendResult.Applied(onePoint.copy(revision = 2))
        tap(model)
        assertTrue(model.ui.value.sending)
        tap(model)
        tap(model, Team.B)
        model.undo()
        scheduler.runCurrent()
        assertEquals(1, api.requests.size)
        api.release()
        scheduler.runCurrent()
        assertFalse(model.ui.value.sending)
        assertEquals(1, api.requests.size)
        assertEquals(listOf("accepted"), haptics.calls)
    }

    @Test
    fun not_sent_buzzes_long_and_keeps_old_view() {
        val model = model()
        api.result = SendResult.NotSent("network error")
        tap(model)
        scheduler.advanceUntilIdle()
        val ui = model.ui.value
        assertSame(onePoint, ui.view)
        assertEquals(Feedback.NotSent, ui.feedback)
        assertFalse(ui.sending)
        assertEquals(listOf("notSent"), haptics.calls)
        assertEquals(1, api.requests.size)
    }

    @Test
    fun timeout_shows_not_sent_and_stream_still_updates_score() {
        val model = model()
        api.result = SendResult.NotSent("network error")
        tap(model)
        val id = model.ui.value.feedbackId
        // The point did land: the server's next frame carries it.
        val landed = fixtureView("view-advantage.json").copy(revision = onePoint.revision + 1)
        emit(StreamItem.Frame(landed))
        val ui = model.ui.value
        assertSame(landed, ui.view)
        assertEquals(Feedback.NotSent, ui.feedback)
        assertEquals(id, ui.feedbackId)
        scheduler.advanceUntilIdle()
        assertEquals(1, api.requests.size)
    }

    @Test
    fun point_after_match_over_shows_match_over_not_not_sent() {
        val model = model()
        api.result = SendResult.Rejected(JsonPrimitive("match_over"))
        tap(model)
        assertEquals(Feedback.MatchOver, model.ui.value.feedback)
        assertSame(onePoint, model.ui.value.view)
        assertEquals(listOf("refused"), haptics.calls)

        api.result = SendResult.NotFound
        tap(model)
        assertEquals(Feedback.MatchOver, model.ui.value.feedback)
        assertEquals(listOf("refused", "refused"), haptics.calls)
    }

    @Test
    fun undo_targets_last_effective_seq() {
        val before = fixtureView("view-undo-before.json") // events [1, 2]
        val model = model(before)
        val after = fixtureView("view-after-undo.json")
        api.result = SendResult.Applied(after)
        model.undo()
        scheduler.runCurrent()
        assertEquals(CommandRequest("id-1", false, before.revision, listOf(Command.Undo(2))), api.requests.single().second)
        assertSame(after, model.ui.value.view)
        assertEquals(Feedback.Accepted, model.ui.value.feedback)
    }

    @Test
    fun undo_with_nothing_to_undo_says_so_without_sending() {
        val model = model(fixtureView("view-new.json"))
        model.undo()
        scheduler.runCurrent()
        assertTrue(api.requests.isEmpty())
        assertEquals(Feedback.NothingToUndo, model.ui.value.feedback)
        assertFalse(model.ui.value.sending)
        assertEquals(listOf("refused"), haptics.calls)
    }

    @Test
    fun send_that_throws_is_not_sent_and_clears_sending() {
        val model = model()
        api.failure = RuntimeException("bug in the client")
        tap(model)
        val ui = model.ui.value
        assertEquals(Feedback.NotSent, ui.feedback)
        assertFalse(ui.sending)
        assertSame(onePoint, ui.view)
        assertEquals(listOf("notSent"), haptics.calls)
        // The next tap goes out: nothing is stuck.
        api.failure = null
        api.result = SendResult.Applied(onePoint.copy(revision = 2))
        tap(model)
        assertEquals(2, api.requests.size)
    }

    @Test
    fun undo_conflict_applies_server_view_and_buzzes_reject() {
        val model = model(fixtureView("view-undo-before.json"))
        val server = fixtureView("view-advantage.json").copy(revision = 5)
        api.result = SendResult.Conflict(server)
        model.undo()
        scheduler.runCurrent()
        val ui = model.ui.value
        assertSame(server, ui.view)
        assertEquals(Feedback.UndoConflict, ui.feedback)
        assertFalse(ui.sending)
        assertEquals(listOf("refused"), haptics.calls)
    }

    @Test
    fun older_stream_frame_is_ignored() {
        val model = model(onePoint.copy(revision = 3))
        emit(StreamItem.Frame(fixtureView("view-advantage.json").copy(revision = 2)))
        emit(StreamItem.Frame(fixtureView("view-advantage.json").copy(revision = 3)))
        assertEquals(onePoint.copy(revision = 3), model.ui.value.view)

        // A POST answer older than what the stream already showed is ignored too.
        api.hold = true
        api.result = SendResult.Applied(fixtureView("view-advantage.json").copy(revision = 4))
        tap(model)
        val newer = onePoint.copy(revision = 5)
        emit(StreamItem.Frame(newer))
        api.release()
        scheduler.runCurrent()
        assertSame(newer, model.ui.value.view)
        assertEquals(Feedback.Accepted, model.ui.value.feedback)
    }

    @Test
    fun live_follows_stream_signals() {
        val model = model()
        assertTrue(model.ui.value.live)
        emit(StreamItem.Offline)
        assertFalse(model.ui.value.live)
        assertSame(onePoint, model.ui.value.view)
        emit(StreamItem.Live)
        assertTrue(model.ui.value.live)
    }

    @Test
    fun restart_after_stop_shows_fresh_view() {
        val model = model()
        model.stopStream()
        scheduler.runCurrent()
        assertEquals(0, stream.subscriptionCount.value)
        // The server moves on while the screen is off; nobody is listening.
        emit(StreamItem.Frame(onePoint.copy(revision = 2)))
        assertSame(onePoint, model.ui.value.view)

        model.startStream()
        model.startStream() // already started: no second collector
        scheduler.runCurrent()
        assertEquals(1, stream.subscriptionCount.value)
        val fresh = fixtureView("view-advantage.json").copy(revision = 3)
        emit(StreamItem.Live)
        emit(StreamItem.Frame(fresh))
        assertSame(fresh, model.ui.value.view)
        assertTrue(model.ui.value.live)
    }

    @Test
    fun unauthorized_from_send_or_stream_sets_flag() {
        val viaSend = model()
        api.result = SendResult.Unauthorized
        tap(viaSend)
        assertTrue(viaSend.unauthorized.value)
        assertFalse(viaSend.ui.value.sending)
        tap(viaSend)
        assertEquals(1, api.requests.size)

        val viaStream = model()
        assertFalse(viaStream.unauthorized.value)
        emit(StreamItem.Unauthorized)
        assertTrue(viaStream.unauthorized.value)
        tap(viaStream)
        assertEquals(1, api.requests.size)
    }

    @Test
    fun finished_match_ignores_points_but_sends_undo() {
        val finished = fixtureView("view-finished.json") // revision 1, last seq 24
        val model = model(finished)
        tap(model)
        tap(model, Team.B)
        assertTrue(api.requests.isEmpty())

        val reopened = fixtureView("view-advantage.json").copy(revision = finished.revision + 1)
        api.result = SendResult.Applied(reopened)
        model.undo()
        scheduler.runCurrent()
        assertEquals(CommandRequest("id-1", false, finished.revision, listOf(Command.Undo(24))), api.requests.single().second)
        assertSame(reopened, model.ui.value.view)
        assertEquals(Feedback.Accepted, model.ui.value.feedback)
    }

    @Test
    fun feedback_id_increments_on_each_feedback() {
        val model = model()
        api.result = SendResult.NotSent("network error")
        tap(model)
        val first = model.ui.value.feedbackId
        tap(model)
        assertEquals(Feedback.NotSent, model.ui.value.feedback)
        assertEquals(first + 1, model.ui.value.feedbackId)
    }

    private companion object {
        const val MATCH = "m1"
    }
}
