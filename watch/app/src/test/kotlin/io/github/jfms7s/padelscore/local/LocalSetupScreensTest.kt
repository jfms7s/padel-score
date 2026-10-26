package io.github.jfms7s.padelscore.local

import androidx.compose.ui.test.assert
import androidx.compose.ui.test.hasScrollAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollToNode
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jfms7s.padelscore.local.engine.DecidingSet
import io.github.jfms7s.padelscore.local.engine.DeuceRule
import io.github.jfms7s.padelscore.local.engine.SetRules
import io.github.jfms7s.padelscore.local.engine.SetsFormat
import io.github.jfms7s.padelscore.local.engine.Start
import io.github.jfms7s.padelscore.net.Player
import io.github.jfms7s.padelscore.net.Team
import io.github.jfms7s.padelscore.score.FakeHaptics
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(AndroidJUnit4::class)
@Config(qualifiers = "w227dp-h227dp-round-watch")
class LocalSetupScreensTest {
    @get:Rule val compose = createComposeRule()

    @Before fun setMain() = Dispatchers.setMain(UnconfinedTestDispatcher())

    @After fun resetMain() = Dispatchers.resetMain()

    private fun model(last: LocalSetup? = null) =
        LocalMatchModel(FakeLocalMatchStore(LocalStoreState(null, last)), FakeHaptics())

    @Test
    fun deuce_pick_calls_next_and_records_it() {
        val model = model()
        var nexts = 0
        compose.setContent { LocalDeuceScreen(model) { nexts++ } }
        compose.onNodeWithText("Golden point").performClick()
        assertEquals(1, nexts)
        assertEquals(DeuceRule.GOLDEN_POINT, model.draft.value.deuce)
    }

    @Test
    fun sets_one_reports_one_so_deciding_is_skipped() {
        val model = model()
        var reported: SetsFormat? = null
        compose.setContent { LocalSetsScreen(model) { reported = it } }
        compose.onNodeWithText("One set").performClick()
        assertEquals(SetsFormat.ONE, reported)
        assertEquals(DecidingSet.FULL_SET, model.draft.value.deciding)
    }

    @Test
    fun last_setup_is_highlighted() {
        val last = LocalSetup(
            SetRules(DeuceRule.STAR_POINT, SetsFormat.BEST_OF_3, DecidingSet.SUPER_TIE_BREAK),
            Start(Player(Team.B, 1), 0),
        )
        val model = model(last)
        compose.setContent { LocalDeuceScreen(model) {} }
        compose.waitForIdle()
        compose.onNodeWithTag(CHOICE_SELECTED_TAG).assert(hasText("Star point"))
    }

    @Test
    fun serve_screen_lists_the_two_teams_and_hint() {
        val model = model()
        compose.setContent { LocalServeScreen(model) {} }
        // The list scrolls on a small screen, so each is scrolled to before it is looked for.
        val list = compose.onNode(hasScrollAction())
        listOf("Blue", "Green", "Not sure? Spin the racket and tap the winner.").forEach {
            list.performScrollToNode(hasText(it))
            compose.onNodeWithText(it).assertExists()
        }
        compose.onNodeWithText("Blue 1").assertDoesNotExist()
    }

    @Test
    fun serve_pick_finishes_setup_with_the_second_players_serving() {
        val model = model()
        model.pickDeuce(DeuceRule.ADVANTAGE)
        model.pickSets(SetsFormat.ONE)
        var done: LocalSetup? = null
        compose.setContent { LocalServeScreen(model) { done = it } }
        compose.onNodeWithText("Green").performClick()
        assertEquals(
            LocalSetup(
                SetRules(DeuceRule.ADVANTAGE, SetsFormat.ONE, DecidingSet.FULL_SET),
                Start(Player(Team.B, 1), 1),
            ),
            done,
        )
        assertEquals(Team.B, model.draft.value.firstTeam)
    }

    @Test
    fun serve_screen_highlights_the_last_setups_team_even_with_an_old_index() {
        val last = LocalSetup(
            SetRules(DeuceRule.ADVANTAGE, SetsFormat.ONE, DecidingSet.FULL_SET),
            Start(Player(Team.B, 0), 0),
        )
        val model = model(last)
        compose.setContent { LocalServeScreen(model) {} }
        compose.waitForIdle()
        compose.onNodeWithTag(CHOICE_SELECTED_TAG).assert(hasText("Green"))
    }

    @Test
    fun serve_screen_highlights_nothing_without_a_last_setup() {
        val model = model()
        compose.setContent { LocalServeScreen(model) {} }
        compose.waitForIdle()
        compose.onNodeWithTag(CHOICE_SELECTED_TAG).assertDoesNotExist()
    }

    @Test
    fun serve_pick_with_a_missing_rule_does_not_finish() {
        val model = model()
        var done: LocalSetup? = null
        compose.setContent { LocalServeScreen(model) { done = it } }
        compose.onNodeWithText("Blue").performClick()
        assertNull(done)
    }
}
