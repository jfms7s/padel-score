package io.github.jfms7s.padelscore.local

import androidx.compose.ui.test.assertHeightIsAtLeast
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jfms7s.padelscore.local.engine.DecidingSet
import io.github.jfms7s.padelscore.local.engine.DeuceRule
import io.github.jfms7s.padelscore.local.engine.SetRules
import io.github.jfms7s.padelscore.local.engine.SetsFormat
import io.github.jfms7s.padelscore.local.engine.Start
import io.github.jfms7s.padelscore.local.engine.replay
import io.github.jfms7s.padelscore.net.Player
import io.github.jfms7s.padelscore.net.Team
import io.github.jfms7s.padelscore.score.Feedback
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

@RunWith(AndroidJUnit4::class)
@Config(qualifiers = "w227dp-h227dp-round-watch")
class LocalEntryTest {
    @get:Rule val compose = createComposeRule()

    private val setup = LocalSetup(
        SetRules(DeuceRule.GOLDEN_POINT, SetsFormat.ONE, DecidingSet.FULL_SET),
        Start(Player(Team.A, 0), 0),
    )

    private fun ui(points: List<Team>?, loaded: Boolean = true): LocalUi {
        val match = points?.let { LocalMatch(setup, it) }
        val score = match?.let { replay(setup.rules, setup.start, it.points) }
        return LocalUi(loaded, match, score, setup, Feedback.None, 0)
    }

    @Test
    fun entry_shows_local_match_when_none() {
        var news = 0
        compose.setContent { LocalEntryButtons(ui(null), onNew = { news++ }, onResume = {}, onDiscard = {}) }
        compose.onNodeWithText("Local match").assertHeightIsAtLeast(48.dp).performClick()
        assertEquals(1, news)
        compose.onNodeWithText("Resume local match").assertDoesNotExist()
    }

    @Test
    fun entry_shows_nothing_until_loaded() {
        compose.setContent { LocalEntryButtons(ui(null, loaded = false), onNew = {}, onResume = {}, onDiscard = {}) }
        compose.onNodeWithText("Local match").assertDoesNotExist()
    }

    @Test
    fun entry_shows_resume_with_summary_and_discard_asks_first() {
        var resumes = 0
        var discards = 0
        val points = listOf(Team.A, Team.A, Team.B)
        compose.setContent {
            LocalEntryButtons(ui(points), onNew = {}, onResume = { resumes++ }, onDiscard = { discards++ })
        }
        compose.onNodeWithText("Resume local match").assertIsDisplayed()
        compose.onNodeWithText("30-15", substring = true).assertExists()
        compose.onNodeWithText("Resume local match").performClick()
        assertEquals(1, resumes)

        compose.onNodeWithText("Discard local match").performClick()
        compose.onNodeWithText("Discard this match?").assertIsDisplayed()
        assertEquals(0, discards)
        compose.onNodeWithText("Cancel").performClick()
        assertEquals(0, discards)
        compose.onNodeWithText("Discard this match?").assertDoesNotExist()

        compose.onNodeWithText("Discard local match").performClick()
        compose.onNodeWithText("Yes, discard").performClick()
        assertEquals(1, discards)
    }

    @Test
    fun finished_entry_shows_final_score() {
        compose.setContent { LocalEntryButtons(ui(List(24) { Team.A }), onNew = {}, onResume = {}, onDiscard = {}) }
        compose.onNodeWithText("6-0").assertIsDisplayed()
    }
}
