package io.github.jfms7s.padelscore.local

import io.github.jfms7s.padelscore.local.engine.DecidingSet
import io.github.jfms7s.padelscore.local.engine.DeuceRule
import io.github.jfms7s.padelscore.local.engine.SetRules
import io.github.jfms7s.padelscore.local.engine.SetsFormat
import io.github.jfms7s.padelscore.local.engine.Start
import io.github.jfms7s.padelscore.net.Current
import io.github.jfms7s.padelscore.net.GameScore
import io.github.jfms7s.padelscore.net.Player
import io.github.jfms7s.padelscore.net.PointCall
import io.github.jfms7s.padelscore.net.Status
import io.github.jfms7s.padelscore.net.Team
import io.github.jfms7s.padelscore.score.FakeHaptics
import io.github.jfms7s.padelscore.score.Feedback
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.test.TestCoroutineScheduler
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class LocalMatchModelTest {
    private val scheduler = TestCoroutineScheduler()
    private val haptics = FakeHaptics()
    private val setup = LocalSetup(
        SetRules(DeuceRule.GOLDEN_POINT, SetsFormat.ONE, DecidingSet.FULL_SET),
        Start(firstServer = Player(Team.A, 0), otherFirstServer = 0),
    )

    /** One set won 6-0 by team A, each game to love. */
    private val finishedPoints = List(24) { Team.A }

    @Before fun setMain() = Dispatchers.setMain(UnconfinedTestDispatcher(scheduler))

    @After fun resetMain() = Dispatchers.resetMain()

    private fun stored(points: List<Team>) = FakeLocalMatchStore(LocalStoreState(LocalMatch(setup, points), setup))

    private fun model(store: FakeLocalMatchStore) = LocalMatchModel(store, haptics).also { scheduler.advanceUntilIdle() }

    private fun storedMatch(store: FakeLocalMatchStore): LocalMatch? = runBlocking { store.state.first().match }

    private fun game(model: LocalMatchModel): GameScore = (model.ui.value.score!!.current as Current.Game).score

    @Test
    fun loads_stored_match_and_replays_it() {
        val m = model(stored(listOf(Team.A, Team.A, Team.B)))
        val ui = m.ui.value
        assertTrue(ui.loaded)
        assertEquals(LocalMatch(setup, listOf(Team.A, Team.A, Team.B)), ui.match)
        assertEquals(setup, ui.lastSetup)
        assertEquals(GameScore(PointCall.THIRTY, PointCall.FIFTEEN, 0, false), game(m))
    }

    @Test
    fun unreplayable_stored_match_is_dropped() {
        val store = stored(finishedPoints + List(3) { Team.B })
        val m = model(store)
        assertTrue(m.ui.value.loaded)
        assertNull(m.ui.value.match)
        assertNull(m.ui.value.score)
        assertNull(storedMatch(store))
    }

    @Test
    fun point_is_saved_then_shown_with_accepted_haptic() {
        val store = stored(emptyList())
        val m = model(store)
        m.point(Team.B)
        scheduler.advanceUntilIdle()
        assertEquals(listOf(LocalMatch(setup, listOf(Team.B))), store.saved)
        assertEquals(listOf(Team.B), m.ui.value.match?.points)
        assertEquals(GameScore(PointCall.LOVE, PointCall.FIFTEEN, 0, false), game(m))
        assertEquals(Feedback.Accepted, m.ui.value.feedback)
        assertEquals(listOf("accepted"), haptics.calls)
    }

    @Test
    fun failed_write_shows_not_saved_and_keeps_score() {
        val store = stored(listOf(Team.A))
        val m = model(store)
        val before = m.ui.value
        store.failWrites = true
        m.point(Team.B)
        scheduler.advanceUntilIdle()
        assertEquals(before.match, m.ui.value.match)
        assertEquals(before.score, m.ui.value.score)
        assertEquals(Feedback.NotSaved, m.ui.value.feedback)
        assertEquals(before.feedbackId + 1, m.ui.value.feedbackId)
        assertEquals(listOf("refused"), haptics.calls)
    }

    @Test
    fun two_quick_points_are_both_saved_in_order() {
        val store = stored(emptyList())
        val m = model(store)
        val gate = CompletableDeferred<Unit>()
        store.gate = gate
        val shown = mutableListOf<Int>()
        val watching = CoroutineScope(UnconfinedTestDispatcher(scheduler)).launch {
            m.ui.collect { shown += it.match?.points?.size ?: -1 }
        }
        m.point(Team.A)
        m.point(Team.B)
        scheduler.advanceUntilIdle()
        assertTrue(store.saved.isEmpty())
        gate.complete(Unit)
        scheduler.advanceUntilIdle()
        watching.cancel()
        assertEquals(listOf(listOf(Team.A), listOf(Team.A, Team.B)), store.saved.map { it.points })
        assertEquals(listOf(Team.A, Team.B), m.ui.value.match?.points)
        assertEquals(listOf("accepted", "accepted"), haptics.calls)
        assertEquals("the shown score never goes backwards", shown.sorted(), shown)
    }

    @Test
    fun a_tap_during_a_later_write_builds_on_every_earlier_point() {
        val store = stored(emptyList())
        val m = model(store)
        val first = CompletableDeferred<Unit>()
        val second = CompletableDeferred<Unit>()
        store.gate = first
        m.point(Team.A)
        m.point(Team.B)
        scheduler.advanceUntilIdle()
        store.gate = second
        first.complete(Unit)
        scheduler.advanceUntilIdle()
        m.point(Team.A)
        scheduler.advanceUntilIdle()
        second.complete(Unit)
        scheduler.advanceUntilIdle()
        assertEquals(
            listOf(listOf(Team.A), listOf(Team.A, Team.B), listOf(Team.A, Team.B, Team.A)),
            store.saved.map { it.points },
        )
        assertEquals(listOf(Team.A, Team.B, Team.A), m.ui.value.match?.points)
    }

    @Test
    fun undo_with_no_points_is_refused() {
        val store = stored(emptyList())
        val m = model(store)
        m.undo()
        scheduler.advanceUntilIdle()
        assertTrue(store.saved.isEmpty())
        assertEquals(Feedback.NothingToUndo, m.ui.value.feedback)
        assertEquals(listOf("refused"), haptics.calls)
    }

    @Test
    fun undo_reopens_a_finished_match() {
        val store = stored(finishedPoints)
        val m = model(store)
        assertEquals(Status.Finished(Team.A), m.ui.value.score?.status)
        m.undo()
        scheduler.advanceUntilIdle()
        assertEquals(finishedPoints.dropLast(1), store.saved.single().points)
        assertEquals(Status.InProgress, m.ui.value.score?.status)
        assertEquals(Feedback.Accepted, m.ui.value.feedback)
        assertEquals(listOf("accepted"), haptics.calls)
    }

    @Test
    fun point_on_finished_match_is_ignored() {
        val store = stored(finishedPoints)
        val m = model(store)
        val before = m.ui.value
        m.point(Team.B)
        scheduler.advanceUntilIdle()
        assertTrue(store.saved.isEmpty())
        assertEquals(before, m.ui.value)
        assertTrue(haptics.calls.isEmpty())
    }

    @Test
    fun finished_match_is_kept_until_discarded() {
        val store = stored(finishedPoints)
        val m = model(store)
        assertEquals(finishedPoints, m.ui.value.match?.points)
        assertEquals(finishedPoints, storedMatch(store)?.points)
        m.discard()
        scheduler.advanceUntilIdle()
        assertNull(m.ui.value.match)
        assertNull(m.ui.value.score)
        assertNull(storedMatch(store))
        assertEquals(setup, m.ui.value.lastSetup)
    }

    @Test
    fun failed_discard_shows_not_saved_and_keeps_match() {
        val store = stored(finishedPoints)
        val m = model(store)
        store.failWrites = true
        m.discard()
        scheduler.advanceUntilIdle()
        assertEquals(finishedPoints, m.ui.value.match?.points)
        assertEquals(Feedback.NotSaved, m.ui.value.feedback)
        assertEquals(listOf("refused"), haptics.calls)
    }

    @Test
    fun play_again_keeps_setup_and_clears_points() {
        val store = stored(finishedPoints)
        val m = model(store)
        m.playAgain()
        scheduler.advanceUntilIdle()
        assertEquals(LocalMatch(setup, emptyList()), store.saved.single())
        assertEquals(LocalMatch(setup, emptyList()), m.ui.value.match)
        assertEquals(Status.InProgress, m.ui.value.score?.status)
    }

    @Test
    fun start_saves_a_new_match_with_no_points() {
        val store = FakeLocalMatchStore()
        val m = model(store)
        assertNull(m.ui.value.match)
        m.start(setup)
        scheduler.advanceUntilIdle()
        assertEquals(LocalMatch(setup, emptyList()), store.saved.single())
        assertEquals(setup, m.ui.value.lastSetup)
        assertEquals(GameScore(PointCall.LOVE, PointCall.LOVE, 0, false), game(m))
    }

    @Test
    fun draft_starts_from_last_setup_and_one_set_skips_deciding() {
        val last = LocalSetup(
            SetRules(DeuceRule.STAR_POINT, SetsFormat.BEST_OF_3, DecidingSet.SUPER_TIE_BREAK),
            Start(firstServer = Player(Team.B, 1), otherFirstServer = 1),
        )
        val m = model(FakeLocalMatchStore(LocalStoreState(null, last)))
        assertEquals(
            SetupDraft(DeuceRule.STAR_POINT, SetsFormat.BEST_OF_3, DecidingSet.SUPER_TIE_BREAK, Team.B),
            m.draft.value,
        )
        m.pickDeuce(DeuceRule.ADVANTAGE)
        m.pickSets(SetsFormat.ONE)
        assertEquals(
            SetupDraft(DeuceRule.ADVANTAGE, SetsFormat.ONE, DecidingSet.FULL_SET, Team.B),
            m.draft.value,
        )
        assertEquals(
            LocalSetup(
                SetRules(DeuceRule.ADVANTAGE, SetsFormat.ONE, DecidingSet.FULL_SET),
                Start(firstServer = Player(Team.A, 1), otherFirstServer = 1),
            ),
            m.pickFirstTeam(Team.A),
        )
        assertEquals(Team.A, m.draft.value.firstTeam)
    }

    @Test
    fun a_stored_match_with_the_old_first_server_index_still_loads_and_replays() {
        val old = LocalSetup(setup.rules, Start(firstServer = Player(Team.A, 0), otherFirstServer = 0))
        val points = listOf(Team.A, Team.A, Team.B)
        val m = model(FakeLocalMatchStore(LocalStoreState(LocalMatch(old, points), old)))
        val ui = m.ui.value
        assertEquals(LocalMatch(old, points), ui.match)
        assertEquals(GameScore(PointCall.THIRTY, PointCall.FIFTEEN, 0, false), game(m))
    }

    @Test
    fun a_last_setup_with_the_old_first_server_index_still_loads() {
        val last = LocalSetup(
            SetRules(DeuceRule.ADVANTAGE, SetsFormat.ONE, DecidingSet.FULL_SET),
            Start(firstServer = Player(Team.B, 0), otherFirstServer = 0),
        )
        val m = model(FakeLocalMatchStore(LocalStoreState(null, last)))
        assertEquals(last, m.ui.value.lastSetup)
        assertEquals(Team.B, m.draft.value.firstTeam)
    }

    @Test
    fun draft_without_last_setup_is_incomplete_until_all_picked() {
        val m = model(FakeLocalMatchStore())
        assertEquals(SetupDraft(null, null, null, null), m.draft.value)
        m.pickDeuce(DeuceRule.GOLDEN_POINT)
        m.pickSets(SetsFormat.BEST_OF_3)
        assertNull(m.pickFirstTeam(Team.B))
        m.pickDeciding(DecidingSet.SUPER_TIE_BREAK)
        assertEquals(
            LocalSetup(
                SetRules(DeuceRule.GOLDEN_POINT, SetsFormat.BEST_OF_3, DecidingSet.SUPER_TIE_BREAK),
                Start(firstServer = Player(Team.B, 1), otherFirstServer = 1),
            ),
            m.pickFirstTeam(Team.B),
        )
    }

    @Test
    fun invalid_last_setup_is_ignored_by_the_draft() {
        val invalid = LocalSetup(
            SetRules(DeuceRule.GOLDEN_POINT, SetsFormat.ONE, DecidingSet.SUPER_TIE_BREAK),
            Start(firstServer = Player(Team.A, 0), otherFirstServer = 0),
        )
        val m = model(FakeLocalMatchStore(LocalStoreState(null, invalid)))
        assertEquals(SetupDraft(null, null, null, null), m.draft.value)
    }
}
