package io.github.jfms7s.padelscore.auth

import androidx.compose.ui.test.assertHeightIsAtLeast
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jfms7s.padelscore.net.TokenPoll
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestCoroutineScheduler
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.setMain
import okhttp3.HttpUrl.Companion.toHttpUrl
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(AndroidJUnit4::class)
class SignInScreenTest {
    @get:Rule val compose = createComposeRule()

    private val scheduler = TestCoroutineScheduler()

    @Before fun setMain() = Dispatchers.setMain(StandardTestDispatcher(scheduler))

    @After fun resetMain() = Dispatchers.resetMain()

    private fun modelWith(poll: TokenPoll): Pair<SignInModel, FakeSignInApi> {
        val api = FakeSignInApi(deviceStart(), mutableListOf(poll), scheduler)
        return SignInModel(api, MemoryTokenStore(), "https://example.test".toHttpUrl(), "SM-R960") to api
    }

    private fun advance(seconds: Long) {
        scheduler.advanceTimeBy(seconds * 1000)
        scheduler.runCurrent()
    }

    @Test
    fun shows_user_code_and_link() {
        val (model, _) = modelWith(TokenPoll.Pending)
        advance(1)
        compose.setContent { SignInScreen(model, onSignedIn = {}) }
        compose.onNodeWithText("Sign in").assertExists()
        compose.onNodeWithText("ABCD-2345").assertExists()
        compose.onNodeWithText("On your phone, open").assertExists()
        compose.onNodeWithText("example.test/link").assertExists()
        compose.onNodeWithText("10 min left").assertExists()
    }

    @Test
    fun expired_shows_new_code_button_which_restarts() {
        val (model, api) = modelWith(TokenPoll.Expired)
        advance(6)
        compose.setContent { SignInScreen(model, onSignedIn = {}) }
        compose.onNodeWithText("Code expired").assertExists()
        compose.onNodeWithText("New code").assertHeightIsAtLeast(48.dp).performClick()
        advance(1)
        compose.waitForIdle()
        assertEquals(2, api.starts)
        compose.onNodeWithText("ABCD-2345").assertExists()
    }

    @Test
    fun signed_in_notifies_the_caller() {
        val (model, _) = modelWith(TokenPoll.Granted("pdl_TEST"))
        advance(6)
        var signedIn = false
        compose.setContent { SignInScreen(model, onSignedIn = { signedIn = true }) }
        compose.waitForIdle()
        assertTrue(signedIn)
    }
}
