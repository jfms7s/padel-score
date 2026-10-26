package io.github.jfms7s.padelscore.local

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jfms7s.padelscore.local.engine.DecidingSet
import io.github.jfms7s.padelscore.local.engine.DeuceRule
import io.github.jfms7s.padelscore.local.engine.SetRules
import io.github.jfms7s.padelscore.local.engine.SetsFormat
import io.github.jfms7s.padelscore.local.engine.Start
import io.github.jfms7s.padelscore.net.Player
import io.github.jfms7s.padelscore.net.Status
import io.github.jfms7s.padelscore.net.Team
import io.github.jfms7s.padelscore.score.FakeHaptics
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(AndroidJUnit4::class)
@Config(qualifiers = "w227dp-h227dp-round-watch")
class LocalScoreScreenTest {
    @get:Rule val compose = createComposeRule()

    private val setup = LocalSetup(
        SetRules(DeuceRule.GOLDEN_POINT, SetsFormat.ONE, DecidingSet.FULL_SET),
        Start(firstServer = Player(Team.A, 0), otherFirstServer = 0),
    )

    @Before fun setMain() = Dispatchers.setMain(UnconfinedTestDispatcher())

    @After fun resetMain() = Dispatchers.resetMain()

    private fun show(store: FakeLocalMatchStore, onBack: () -> Unit = {}): LocalMatchModel {
        val model = LocalMatchModel(store, FakeHaptics())
        compose.setContent { LocalScoreScreen(model, onBack) }
        compose.waitForIdle()
        return model
    }

    private fun stored(points: List<Team>) = FakeLocalMatchStore(LocalStoreState(LocalMatch(setup, points), setup))

    private fun storedMatch(store: FakeLocalMatchStore): LocalMatch? = runBlocking { store.state.first().match }

    @Test
    fun shows_local_chip_and_no_offline_chip() {
        show(stored(emptyList()))
        compose.onNodeWithText("Local match").assertIsDisplayed()
        compose.onNodeWithText("Quick match").assertDoesNotExist()
        compose.onNodeWithText("Offline").assertDoesNotExist()
    }

    @Test
    fun tap_left_half_scores_blue() {
        show(stored(emptyList()))
        compose.onNodeWithContentDescription("Point Blue").performClick()
        compose.waitForIdle()
        compose.onNodeWithText("15").assertIsDisplayed()
    }

    @Test
    fun finished_shows_play_again_and_back() {
        val store = stored(List(24) { Team.A })
        var backs = 0
        val model = show(store) { backs++ }
        compose.onNodeWithText("Play again").assertIsDisplayed()
        compose.onNodeWithText("Back").assertIsDisplayed()

        compose.onNodeWithText("Play again").performClick()
        compose.waitForIdle()
        assertEquals(emptyList<Team>(), model.ui.value.match?.points)
        assertTrue(model.ui.value.score?.status == Status.InProgress)
        assertEquals(0, backs)
    }

    @Test
    fun back_discards_the_match_and_leaves() {
        val store = stored(List(24) { Team.A })
        var backs = 0
        val model = show(store) { backs++ }
        compose.onNodeWithText("Back").performClick()
        compose.waitForIdle()
        assertEquals(1, backs)
        assertNull(storedMatch(store))
        assertNull(model.ui.value.match)
    }

    @Test
    fun not_saved_notice_shows_when_write_fails() {
        val store = stored(emptyList())
        show(store)
        store.failWrites = true
        compose.onNodeWithContentDescription("Point Blue").performClick()
        compose.waitForIdle()
        compose.onNodeWithText("Not saved").assertIsDisplayed()
    }

    @Test
    fun golden_point_chip_shows_at_40_40() {
        show(stored(listOf(Team.A, Team.B, Team.A, Team.B, Team.A, Team.B)))
        compose.onNodeWithText("Golden point").assertIsDisplayed()
    }
}
