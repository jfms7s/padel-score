package io.github.jfms7s.padelscore

import androidx.activity.OnBackPressedDispatcher
import androidx.activity.compose.LocalOnBackPressedDispatcherOwner
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.lifecycle.ViewModelProvider
import androidx.navigation.NavHostController
import androidx.wear.compose.navigation.rememberSwipeDismissableNavController
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jfms7s.padelscore.auth.TokenStore
import io.github.jfms7s.padelscore.haptics.Haptics
import io.github.jfms7s.padelscore.net.ApiResult
import io.github.jfms7s.padelscore.net.CommandRequest
import io.github.jfms7s.padelscore.net.DeviceStart
import io.github.jfms7s.padelscore.net.MatchView
import io.github.jfms7s.padelscore.net.MyMatch
import io.github.jfms7s.padelscore.net.PadelApi
import io.github.jfms7s.padelscore.net.SendResult
import io.github.jfms7s.padelscore.net.StreamItem
import io.github.jfms7s.padelscore.net.TokenPoll
import io.github.jfms7s.padelscore.score.FakeHaptics
import io.github.jfms7s.padelscore.score.fixtureView
import io.github.jfms7s.padelscore.settings.FakeSettingsStore
import io.github.jfms7s.padelscore.settings.ServerSettings
import io.github.jfms7s.padelscore.settings.SettingsModel
import io.github.jfms7s.padelscore.settings.testDefaults
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.emptyFlow
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.runBlocking
import okhttp3.Request
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

private class FakeTokens(token: String?) : TokenStore {
    private val stored = MutableStateFlow(token)
    var clears = 0

    override suspend fun read(): String? = stored.value
    override suspend fun write(token: String) {
        stored.value = token
    }
    override suspend fun clear() {
        clears++
        stored.value = null
    }
    override val signedIn: Flow<Boolean> = stored.map { it != null }
}

private class FakeApi(
    var matches: ApiResult<List<MyMatch>>,
    var sendResult: SendResult = SendResult.NotSent("unset"),
) : PadelApi {
    var starts = 0
    var gets = 0

    // Fails at once so the sign-in screen settles on a stable state ("New code").
    override suspend fun startDevice(deviceName: String): ApiResult<DeviceStart> {
        starts++
        return ApiResult.Failed("offline")
    }

    override suspend fun pollToken(deviceCode: String): TokenPoll = TokenPoll.Pending

    override suspend fun myMatches(): ApiResult<List<MyMatch>> {
        gets++
        return matches
    }

    override suspend fun send(matchId: String, request: CommandRequest): SendResult = sendResult
    override suspend fun listStreamRequest(): Request = error("unused")
    override suspend fun matchStreamRequest(matchId: String): Request = error("unused")
}

private class FakeContainer(
    override val api: FakeApi,
    override val tokens: FakeTokens,
    private val list: Flow<StreamItem<List<MyMatch>>> = emptyFlow(),
    private val match: Flow<StreamItem<MatchView>> = emptyFlow(),
    override val settingsStore: FakeSettingsStore = FakeSettingsStore(),
    override val settings: ServerSettings = testDefaults,
) : AppContainer {
    override val haptics: Haptics = FakeHaptics()
    override val deviceName: String = "Test watch"
    var matchStreams = 0

    override fun listStream(): Flow<StreamItem<List<MyMatch>>> = list
    override fun matchStream(matchId: String): Flow<StreamItem<MatchView>> {
        matchStreams++
        return match
    }
}

@RunWith(AndroidJUnit4::class)
@Config(qualifiers = "w227dp-h227dp-round-watch")
class WatchAppTest {
    @get:Rule val compose = createComposeRule()

    private val view = fixtureView("view-one-point.json")
    private val oneMatch = ApiResult.Ok(listOf(MyMatch(view.id, "Ana / Rui v João / Marta", view)))

    private lateinit var nav: NavHostController
    private lateinit var back: OnBackPressedDispatcher
    private var applied = 0

    private fun launch(container: AppContainer) {
        compose.setContent {
            nav = rememberSwipeDismissableNavController()
            back = checkNotNull(LocalOnBackPressedDispatcherOwner.current).onBackPressedDispatcher
            WatchApp(container, nav, onSettingsApplied = { applied++ })
        }
        compose.waitForIdle()
    }

    @Test
    fun starts_on_matches_when_signed_in_else_sign_in() {
        val signedIn = FakeContainer(FakeApi(oneMatch), FakeTokens("tok"))
        val signedOut = FakeContainer(FakeApi(oneMatch), FakeTokens(null))
        var current by mutableStateOf<AppContainer>(signedIn)
        compose.setContent { key(current) { WatchApp(current, onSettingsApplied = {}) } }

        compose.onNodeWithText("Ana / Rui v João / Marta").assertExists()
        compose.onNodeWithText("New code").assertDoesNotExist()
        assertEquals(0, signedIn.api.starts)

        current = signedOut // a fresh app process with no stored token
        compose.onNodeWithText("New code").assertExists()
        compose.onNodeWithText("Sign out").assertDoesNotExist()
        assertEquals(0, signedOut.api.gets)
    }

    @Test
    fun unauthorized_clears_token_and_returns_to_sign_in() {
        // Mid-match revoke: the next point answers 401.
        val api = FakeApi(oneMatch, sendResult = SendResult.Unauthorized)
        val tokens = FakeTokens("tok")
        val container = FakeContainer(api, tokens, match = flowOf(StreamItem.Live, StreamItem.Frame(view)))
        launch(container)
        compose.onNodeWithText("Ana / Rui v João / Marta").performClick()
        compose.onNodeWithContentDescription("Point Ana / Rui").performClick()
        compose.waitForIdle()

        compose.onNodeWithText("New code").assertExists()
        assertEquals(1, tokens.clears)
        assertNull(runBlocking { tokens.read() })
        assertEquals(1, api.starts)
        assertEquals(1, container.matchStreams) // no reconnect after the 401
        assertEquals("signin", nav.currentDestination?.route)
        assertNull(nav.previousBackStackEntry) // nothing to swipe back to
    }

    @Test
    fun list_unauthorized_clears_token_and_returns_to_sign_in() {
        val api = FakeApi(ApiResult.Unauthorized)
        val tokens = FakeTokens("tok")
        launch(FakeContainer(api, tokens, list = flowOf(StreamItem.Unauthorized)))

        compose.onNodeWithText("New code").assertExists()
        assertEquals(1, tokens.clears) // the GET and the stream both said 401: signed out once
        assertEquals(1, api.gets)
        assertNull(nav.previousBackStackEntry)
    }

    @Test
    fun stream_unauthorized_on_score_returns_to_sign_in() {
        val api = FakeApi(oneMatch)
        val tokens = FakeTokens("tok")
        launch(FakeContainer(api, tokens, match = flowOf(StreamItem.Frame(view), StreamItem.Unauthorized)))
        compose.onNodeWithText("Ana / Rui v João / Marta").performClick()
        compose.waitForIdle()

        compose.onNodeWithText("New code").assertExists()
        assertEquals(1, tokens.clears)
        assertNull(nav.previousBackStackEntry)
    }

    @Test
    fun list_stream_closed_under_score_and_reopened_after_back() {
        val list = MutableSharedFlow<StreamItem<List<MyMatch>>>(extraBufferCapacity = 4)
        val api = FakeApi(oneMatch)
        launch(FakeContainer(api, FakeTokens("tok"), list = list, match = flowOf(StreamItem.Frame(view))))
        assertEquals(1, list.subscriptionCount.value)

        compose.onNodeWithText("Ana / Rui v João / Marta").performClick()
        compose.onNodeWithContentDescription("Point Ana / Rui").assertExists()
        // Only the match stream is open during the match.
        assertEquals(0, list.subscriptionCount.value)

        compose.runOnUiThread { back.onBackPressed() }
        compose.waitForIdle()
        assertEquals(1, list.subscriptionCount.value)
        assertEquals(1, api.gets) // coming back reconnects the stream, not the GET
    }

    @Test
    fun sign_out_from_list_clears_token_and_returns_to_sign_in() {
        val tokens = FakeTokens("tok")
        launch(FakeContainer(FakeApi(oneMatch), tokens))
        compose.onNodeWithText("Sign out").performClick()
        compose.onNodeWithText("Yes, sign out").performClick()
        compose.waitForIdle()

        compose.onNodeWithText("New code").assertExists()
        assertEquals(1, tokens.clears)
        assertNull(nav.previousBackStackEntry)
    }

    @Test
    fun swipe_right_from_score_returns_to_matches() {
        // On Wear OS 6 (API 36, the Robolectric SDK here) the system turns the swipe into a back event.
        launch(FakeContainer(FakeApi(oneMatch), FakeTokens("tok"), match = flowOf(StreamItem.Frame(view))))
        compose.onNodeWithText("Ana / Rui v João / Marta").performClick()
        compose.onNodeWithContentDescription("Point Ana / Rui").assertExists()

        compose.runOnUiThread { back.onBackPressed() }
        compose.waitForIdle()

        assertEquals("matches", nav.currentDestination?.route)
        compose.onNodeWithText("Ana / Rui v João / Marta").assertExists()
    }

    @Test
    fun back_to_matches_from_finished_score() {
        val finished = fixtureView("view-finished.json")
        launch(
            FakeContainer(
                FakeApi(ApiResult.Ok(listOf(MyMatch(finished.id, "Ana / Rui v João / Marta", finished)))),
                FakeTokens("tok"),
                match = flowOf(StreamItem.Frame(finished)),
            ),
        )
        compose.onNodeWithText("Ana / Rui v João / Marta").performClick()
        compose.onNodeWithText("Back to matches").performClick()
        compose.waitForIdle()
        assertEquals("matches", nav.currentDestination?.route)
    }

    // The restart itself (a fresh activity, old models cleared) is covered by MainActivityTest.
    @Test
    fun new_server_clears_token_and_asks_for_a_restart() {
        val tokens = FakeTokens("tok")
        val container = FakeContainer(FakeApi(oneMatch), tokens)
        launch(container)
        compose.onNodeWithText("Server settings").performClick()
        compose.onNodeWithText("example.test").assertExists()

        submit { it.submitServer("padel.other.test") }

        assertEquals(1, applied)
        assertEquals(1, tokens.clears)
        assertNull(runBlocking { tokens.read() })
        assertEquals("https://padel.other.test/", container.settingsStore.server.toString())
    }

    @Test
    fun new_addresses_keep_the_sign_in() {
        val tokens = FakeTokens("tok")
        val container = FakeContainer(FakeApi(oneMatch), tokens)
        launch(container)
        compose.onNodeWithText("Server settings").performClick()

        submit { it.submitAddresses("10.1.2.3, 10.1.2.4") }

        assertEquals(1, applied)
        assertEquals(0, tokens.clears)
        assertEquals("tok", runBlocking { tokens.read() })
        assertEquals(listOf("10.1.2.3", "10.1.2.4"), container.settingsStore.fallback?.map { it.hostAddress })
    }

    @Test
    fun settings_from_sign_in_and_back() {
        launch(FakeContainer(FakeApi(oneMatch), FakeTokens(null)))
        compose.onNodeWithText("Server settings").performClick()
        compose.onNodeWithText("LAN addresses").assertExists()

        compose.runOnUiThread { back.onBackPressed() }
        compose.waitForIdle()
        assertEquals("signin", nav.currentDestination?.route)
    }

    /** What the keyboard would hand back, given straight to the settings screen's model. */
    private fun submit(action: (SettingsModel) -> Unit) {
        compose.waitForIdle()
        val entry = checkNotNull(nav.currentBackStackEntry)
        assertEquals("settings", entry.destination.route)
        val model = ViewModelProvider.create(entry)[SettingsModel::class]
        compose.runOnUiThread { action(model) }
        compose.waitForIdle()
    }
}
