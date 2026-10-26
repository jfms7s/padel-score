package io.github.jfms7s.padelscore.local

import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertIsDisplayed
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
    fun serve_screen_lists_four_players_and_hint() {
        val model = model()
        var nexts = 0
        compose.setContent { LocalServeScreen(model) { nexts++ } }
        // The list scrolls on a small screen, so each is scrolled to before it is looked for.
        val list = compose.onNode(hasScrollAction())
        listOf("Blue 1", "Blue 2", "Green 1", "Green 2", "Not sure? Spin the racket and tap the winner.").forEach {
            list.performScrollToNode(hasText(it))
            compose.onNodeWithText(it).assertExists()
        }
        list.performScrollToNode(hasText("Green 1"))
        compose.onNodeWithText("Green 1").performClick()
        assertEquals(Player(Team.B, 0), model.draft.value.firstServer)
        assertEquals(1, nexts)
    }

    @Test
    fun other_server_lists_the_other_team_and_finishes_setup() {
        val model = model()
        model.pickDeuce(DeuceRule.ADVANTAGE)
        model.pickSets(SetsFormat.ONE)
        model.pickFirstServer(Player(Team.A, 1))
        var done: LocalSetup? = null
        compose.setContent { LocalOtherServerScreen(model) { done = it } }
        compose.onNodeWithText("Green 1").assertIsDisplayed()
        compose.onNodeWithText("Blue 1").assertDoesNotExist()
        compose.onNodeWithText("Green 2").performClick()
        assertEquals(
            LocalSetup(
                SetRules(DeuceRule.ADVANTAGE, SetsFormat.ONE, DecidingSet.FULL_SET),
                Start(Player(Team.A, 1), 1),
            ),
            done,
        )
    }

    @Test
    fun other_server_with_a_missing_pick_does_not_finish() {
        val model = model()
        model.pickFirstServer(Player(Team.A, 0))
        var done: LocalSetup? = null
        compose.setContent { LocalOtherServerScreen(model) { done = it } }
        compose.onNodeWithText("Green 1").performClick()
        assertNull(done)
    }
}
