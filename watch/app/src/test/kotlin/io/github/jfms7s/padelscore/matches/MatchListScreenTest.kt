package io.github.jfms7s.padelscore.matches

import androidx.compose.ui.test.assertHeightIsAtLeast
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jfms7s.padelscore.net.ApiResult
import io.github.jfms7s.padelscore.net.MyMatch
import io.github.jfms7s.padelscore.net.StreamItem
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.emptyFlow
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestCoroutineScheduler
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(AndroidJUnit4::class)
class MatchListScreenTest {
    @get:Rule val compose = createComposeRule()

    private val scheduler = TestCoroutineScheduler()

    @Before fun setMain() = Dispatchers.setMain(StandardTestDispatcher(scheduler))

    @After fun resetMain() = Dispatchers.resetMain()

    private fun modelOf(
        matches: List<MyMatch>,
        stream: Flow<StreamItem<List<MyMatch>>> = emptyFlow(),
    ): MatchListModel {
        val model = MatchListModel(FakeListApi(ApiResult.Ok(matches)), stream, "Quick match")
        scheduler.runCurrent()
        return model
    }

    @Test
    fun empty_list_shows_hint() {
        compose.setContent { MatchListScreen(modelOf(emptyList()), onPick = {}, onSignOut = {}) }
        compose.onNodeWithText("No live matches. Start one on your phone.").assertExists()
    }

    @Test
    fun tapping_row_picks_its_id() {
        val model = modelOf(listOf(MyMatch("m1", "Ana / Rui v João / Marta")))
        var picked: String? = null
        compose.setContent { MatchListScreen(model, onPick = { picked = it }, onSignOut = {}) }
        compose.onNodeWithText("Quick match").assertHeightIsAtLeast(48.dp)
        compose.onNodeWithText("Ana / Rui v João / Marta").performClick()
        assertEquals("m1", picked)
    }

    @Test
    fun sign_out_needs_confirm() {
        var signedOut = 0
        compose.setContent { MatchListScreen(modelOf(emptyList()), onPick = {}, onSignOut = { signedOut++ }) }
        compose.onNodeWithText("Sign out").assertHeightIsAtLeast(48.dp).performClick()
        assertEquals(0, signedOut)
        compose.onNodeWithText("Signing out only forgets this watch", substring = true).assertExists()
        compose.onNodeWithText("Yes, sign out").performClick()
        assertEquals(1, signedOut)
    }

    @Test
    fun cancel_keeps_the_sign_in() {
        var signedOut = 0
        compose.setContent { MatchListScreen(modelOf(emptyList()), onPick = {}, onSignOut = { signedOut++ }) }
        compose.onNodeWithText("Sign out").performClick()
        compose.onNodeWithText("Cancel").performClick()
        assertEquals(0, signedOut)
    }
}
