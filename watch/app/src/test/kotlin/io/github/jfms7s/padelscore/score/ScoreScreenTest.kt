package io.github.jfms7s.padelscore.score

import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertHeightIsAtLeast
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jfms7s.padelscore.net.Command
import io.github.jfms7s.padelscore.net.MatchView
import io.github.jfms7s.padelscore.net.SendResult
import io.github.jfms7s.padelscore.net.StreamItem
import io.github.jfms7s.padelscore.net.Team
import kotlinx.coroutines.flow.flowOf
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

@RunWith(AndroidJUnit4::class)
@Config(qualifiers = "w227dp-h227dp-round-watch")
class ScoreScreenTest {
    @get:Rule val compose = createComposeRule()

    private val api = FakeSendApi()
    private val haptics = FakeHaptics()

    /** Shows [view] through the stream; [live] says whether the stream reported itself connected. */
    private fun show(view: MatchView, live: Boolean = true, onDone: () -> Unit = {}): ScoreModel {
        val items = if (live) listOf(StreamItem.Live, StreamItem.Frame(view)) else listOf(StreamItem.Frame(view))
        val model = ScoreModel(MATCH, api, flowOf(*items.toTypedArray()), haptics)
        compose.setContent { ScoreScreen(model, onDone) }
        compose.waitForIdle()
        return model
    }

    private fun sentCommands(): List<Command> = api.requests.flatMap { it.second.commands }

    /** Holds Undo past its 600 ms threshold; needs `mainClock.autoAdvance = false`. */
    private fun holdUndo() {
        val undo = compose.onNodeWithText("Undo")
        undo.performTouchInput { down(center) }
        compose.mainClock.advanceTimeBy(700)
        undo.performTouchInput { up() }
        compose.mainClock.advanceTimeByFrame()
    }

    @Test
    fun tapping_left_half_sends_point_a() {
        val view = fixtureView("view-one-point.json")
        api.result = SendResult.Applied(view.copy(revision = 2))
        show(view)
        compose.onNodeWithContentDescription("Point Ana / Rui").performClick()
        assertEquals(listOf(Command.Point(Team.A)), sentCommands())
        assertEquals(MATCH, api.requests.single().first)
    }

    @Test
    fun tapping_right_half_sends_point_b() {
        val view = fixtureView("view-one-point.json")
        api.result = SendResult.Applied(view.copy(revision = 2))
        show(view)
        compose.onNodeWithContentDescription("Point João / Marta").performClick()
        assertEquals(listOf(Command.Point(Team.B)), sentCommands())
    }

    @Test
    fun short_press_on_undo_does_nothing_long_press_sends_undo() {
        val view = fixtureView("view-undo-before.json") // last effective seq 2
        api.result = SendResult.Applied(fixtureView("view-after-undo.json"))
        show(view)
        compose.mainClock.autoAdvance = false
        val undo = compose.onNodeWithText("Undo")

        undo.performTouchInput { down(center) }
        compose.mainClock.advanceTimeBy(200)
        undo.performTouchInput { up() }
        compose.mainClock.advanceTimeBy(1_000)
        assertTrue(api.requests.isEmpty())

        undo.performTouchInput { down(center) }
        compose.mainClock.advanceTimeBy(700)
        undo.performTouchInput { up() }
        compose.mainClock.advanceTimeByFrame()
        assertEquals(listOf(Command.Undo(2)), sentCommands())
    }

    @Test
    fun not_sent_shows_icon_and_text() {
        val view = fixtureView("view-one-point.json")
        api.result = SendResult.NotSent("network error")
        show(view)
        compose.mainClock.autoAdvance = false
        compose.onNodeWithContentDescription("Point Ana / Rui").performClick()
        compose.mainClock.advanceTimeByFrame()
        compose.onNodeWithText("Not sent").assertExists()
        compose.onNodeWithTag(NOT_SENT_ICON_TAG, useUnmergedTree = true).assertExists()
        // The view is unchanged: nothing was counted locally.
        compose.onNodeWithContentDescription("Point Ana / Rui").assert(hasText("15"))

        compose.mainClock.advanceTimeBy(3_100)
        compose.onNodeWithText("Not sent").assertDoesNotExist()
    }

    @Test
    fun finished_match_shows_winner_and_ignores_taps() {
        var done = 0
        show(fixtureView("view-finished.json"), onDone = { done++ })
        compose.onNodeWithText("Ana / Rui win").assertExists()
        compose.onNodeWithContentDescription("Point Ana / Rui").assertIsNotEnabled().performClick()
        compose.onNodeWithContentDescription("Point João / Marta").assertIsNotEnabled().performClick()
        assertTrue(api.requests.isEmpty())
        compose.onNodeWithText("Back to matches").performClick()
        assertEquals(1, done)
    }

    @Test
    fun finished_match_keeps_undo_below_banner_and_back() {
        show(fixtureView("view-finished.json"))
        val banner = compose.onNodeWithText("Ana / Rui win").getUnclippedBoundsInRoot()
        val back = compose.onNodeWithText("Back to matches").assertHeightIsAtLeast(48.dp).getUnclippedBoundsInRoot()
        val undo = compose.onNodeWithText("Undo").assertHeightIsAtLeast(48.dp).getUnclippedBoundsInRoot()
        assertTrue("banner above back", banner.bottom <= back.top)
        assertTrue("back above undo", back.bottom <= undo.top)
        assertTrue("undo on screen", undo.bottom <= 227.dp)
    }

    @Test
    fun long_press_undo_on_finished_match_sends_undo_and_reopened_view_drops_banner() {
        val finished = fixtureView("view-finished.json") // revision 1, last effective seq 24
        api.result = SendResult.Applied(fixtureView("view-one-point.json").copy(revision = finished.revision + 1))
        show(finished)
        compose.mainClock.autoAdvance = false
        holdUndo()
        assertEquals(listOf(Command.Undo(24)), sentCommands())
        compose.onNodeWithText("Ana / Rui win").assertDoesNotExist()
        compose.onNodeWithText("Back to matches").assertDoesNotExist()
        compose.onNodeWithContentDescription("Point Ana / Rui").assertIsEnabled().assert(hasText("15"))
    }

    @Test
    fun notice_on_finished_match_takes_banner_place_clear_of_undo() {
        api.result = SendResult.NotSent("network error")
        show(fixtureView("view-finished.json"))
        compose.mainClock.autoAdvance = false
        holdUndo()
        compose.onNodeWithText("Not sent").assertExists()
        compose.onNodeWithText("Ana / Rui win").assertDoesNotExist()
        val back = compose.onNodeWithText("Back to matches").getUnclippedBoundsInRoot()
        val undo = compose.onNodeWithText("Undo").getUnclippedBoundsInRoot()
        assertTrue("back above undo", back.bottom <= undo.top)
        compose.mainClock.advanceTimeBy(3_100)
        compose.onNodeWithText("Not sent").assertDoesNotExist()
        compose.onNodeWithText("Ana / Rui win").assertExists()
    }

    @Test
    fun match_over_notice_clears_after_three_seconds() {
        api.result = SendResult.NotFound
        show(fixtureView("view-one-point.json"))
        compose.mainClock.autoAdvance = false
        compose.onNodeWithContentDescription("Point Ana / Rui").performClick()
        compose.mainClock.advanceTimeByFrame()
        compose.onNodeWithText("Match over").assertExists()
        compose.mainClock.advanceTimeBy(3_100)
        compose.onNodeWithText("Match over").assertDoesNotExist()
    }

    @Test
    fun undo_with_nothing_to_undo_shows_notice_then_clears() {
        show(fixtureView("view-new.json"))
        compose.mainClock.autoAdvance = false
        holdUndo()
        assertTrue(api.requests.isEmpty())
        compose.onNodeWithText("Nothing to undo").assertExists()
        compose.mainClock.advanceTimeBy(3_100)
        compose.onNodeWithText("Nothing to undo").assertDoesNotExist()
    }

    @Test
    fun offline_chip_when_stream_down() {
        val view = fixtureView("view-one-point.json")
        api.result = SendResult.NotSent("network error")
        show(view, live = false)
        compose.onNodeWithText("Offline").assertExists()
        // Taps still try: the stream may be down while the network is fine.
        compose.onNodeWithContentDescription("Point Ana / Rui").performClick()
        assertEquals(1, api.requests.size)
        compose.onNodeWithText("Not sent").assertExists()
    }

    @Test
    fun online_has_no_offline_chip() {
        show(fixtureView("view-one-point.json"))
        compose.onNodeWithText("Offline").assertDoesNotExist()
    }

    @Test
    fun shows_points_games_and_names_from_view() {
        // view-advantage.json: team A (left) has the advantage over B's forty.
        show(fixtureView("view-advantage.json"))
        compose.onNodeWithContentDescription("Point Ana / Rui")
            .assert(hasText("AD"))
            .assert(hasText("Ana / Rui"))
            .assert(hasText("G0 S0"))
        compose.onNodeWithContentDescription("Point João / Marta")
            .assert(hasText("40"))
            .assert(hasText("João / Marta"))
            .assert(hasText("G0 S0"))
        compose.onNodeWithText("Quick match").assertExists()
        compose.onNodeWithTag(SERVE_BALL_TAG, useUnmergedTree = true).assertExists()
    }

    @Test
    fun golden_point_and_change_ends_chips() {
        show(fixtureView("view-golden-point-40-40.json"))
        compose.onNodeWithText("Golden point").assertExists()
        compose.onNodeWithText("Change ends").assertDoesNotExist()
    }

    @Test
    fun star_point_shows_deciding_point_chip() {
        show(fixtureView("view-star-point.json"))
        compose.onNodeWithText("Deciding point").assertExists()
    }

    @Test
    fun change_ends_chip() {
        show(fixtureView("view-change-ends.json"))
        compose.onNodeWithText("Change ends").assertExists()
    }

    private companion object {
        const val MATCH = "m1"
    }
}
