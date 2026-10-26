package io.github.jfms7s.padelscore

import androidx.activity.OnBackPressedDispatcher
import androidx.activity.compose.LocalOnBackPressedDispatcherOwner
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.getBoundsInRoot
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.ViewModelStoreOwner
import androidx.lifecycle.viewmodel.compose.LocalViewModelStoreOwner
import androidx.navigation.NavHostController
import androidx.wear.compose.navigation.rememberSwipeDismissableNavController
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jfms7s.padelscore.auth.TokenStore
import io.github.jfms7s.padelscore.haptics.Haptics
import io.github.jfms7s.padelscore.local.FakeLocalMatchStore
import io.github.jfms7s.padelscore.local.LocalDeps
import io.github.jfms7s.padelscore.local.LocalMatch
import io.github.jfms7s.padelscore.local.LocalMatchModel
import io.github.jfms7s.padelscore.local.LocalSetup
import io.github.jfms7s.padelscore.local.LocalStoreState
import io.github.jfms7s.padelscore.local.engine.DecidingSet
import io.github.jfms7s.padelscore.local.engine.DeuceRule
import io.github.jfms7s.padelscore.local.engine.SetRules
import io.github.jfms7s.padelscore.local.engine.SetsFormat
import io.github.jfms7s.padelscore.local.engine.Start
import io.github.jfms7s.padelscore.net.ApiResult
import io.github.jfms7s.padelscore.net.CommandRequest
import io.github.jfms7s.padelscore.net.DeviceStart
import io.github.jfms7s.padelscore.net.MatchView
import io.github.jfms7s.padelscore.net.MyMatch
import io.github.jfms7s.padelscore.net.PadelApi
import io.github.jfms7s.padelscore.net.Player
import io.github.jfms7s.padelscore.net.SendResult
import io.github.jfms7s.padelscore.net.StreamItem
import io.github.jfms7s.padelscore.net.Team
import io.github.jfms7s.padelscore.net.TokenPoll
import io.github.jfms7s.padelscore.score.FakeHaptics
import io.github.jfms7s.padelscore.score.fixtureView
import io.github.jfms7s.padelscore.settings.FakeSettingsStore
import io.github.jfms7s.padelscore.settings.ServerSettings
import io.github.jfms7s.padelscore.settings.SettingsModel
import io.github.jfms7s.padelscore.settings.testSettings
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.emptyFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.runBlocking
import okhttp3.Request
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
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
    override val settings: ServerSettings = testSettings,
    override val settingsStore: FakeSettingsStore = FakeSettingsStore(settings.serverUrl, settings.fallback),
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

    private val setup = LocalSetup(
        SetRules(DeuceRule.GOLDEN_POINT, SetsFormat.ONE, DecidingSet.FULL_SET),
        Start(Player(Team.A, 0), 1),
    )

    private lateinit var nav: NavHostController
    private lateinit var back: OnBackPressedDispatcher
    private lateinit var owner: ViewModelStoreOwner
    private var applied = 0

    private fun launch(container: AppContainer?, store: FakeLocalMatchStore = FakeLocalMatchStore()) {
        compose.setContent {
            nav = rememberSwipeDismissableNavController()
            back = checkNotNull(LocalOnBackPressedDispatcherOwner.current).onBackPressedDispatcher
            owner = checkNotNull(LocalViewModelStoreOwner.current)
            WatchApp(container, FakeSettingsStore(), LocalDeps(store, FakeHaptics()), nav, onSettingsApplied = { applied++ })
        }
        compose.waitForIdle()
    }

    private fun stored(points: List<Team>) = FakeLocalMatchStore(LocalStoreState(LocalMatch(setup, points), setup))

    private fun storedMatch(store: FakeLocalMatchStore): LocalMatch? = runBlocking { store.state.first().match }

    private fun pressBack() {
        compose.runOnUiThread { back.onBackPressed() }
        compose.waitForIdle()
    }

    private fun tap(text: String) {
        compose.onNodeWithText(text).performClick()
        compose.waitForIdle()
    }

    private fun signedOut() = FakeContainer(FakeApi(oneMatch), FakeTokens(null))

    @Test
    fun starts_on_matches_when_signed_in_else_sign_in() {
        val signedIn = FakeContainer(FakeApi(oneMatch), FakeTokens("tok"))
        val signedOut = FakeContainer(FakeApi(oneMatch), FakeTokens(null))
        var current by mutableStateOf<AppContainer>(signedIn)
        val local = LocalDeps(FakeLocalMatchStore(), FakeHaptics())
        compose.setContent { key(current) { WatchApp(current, FakeSettingsStore(), local, onSettingsApplied = {}) } }

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
        compose.onNodeWithText("Sign out").performScrollTo().performClick() // below the local match entry
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
        assertEquals(listOf("10.1.2.3", "10.1.2.4"), container.settingsStore.fallback.map { it.hostAddress })
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

    @Test
    fun no_server_shows_first_run_with_local_match_entry() {
        launch(null)
        assertEquals("first-run", nav.currentDestination?.route)
        compose.onNodeWithText("Not set").assertExists()
        tap("Local match")
        assertEquals("local/deuce", nav.currentDestination?.route)
    }

    @Test
    fun signed_out_shows_local_match_on_sign_in() {
        launch(signedOut())
        compose.onNodeWithText("New code").assertExists()
        tap("Local match")
        assertEquals("local/deuce", nav.currentDestination?.route)
    }

    @Test
    fun match_list_has_local_match_on_top() {
        launch(FakeContainer(FakeApi(oneMatch), FakeTokens("tok")))
        val entry = compose.onNodeWithText("Local match").getBoundsInRoot()
        val row = compose.onNodeWithText("Ana / Rui v João / Marta").getBoundsInRoot()
        assertTrue(entry.bottom <= row.top)
    }

    @Test
    fun local_setup_one_set_skips_deciding_and_opens_score() {
        val store = FakeLocalMatchStore()
        launch(signedOut(), store)
        tap("Local match")
        tap("Golden point")
        tap("One set")
        assertEquals("local/serve", nav.currentDestination?.route)
        tap("Blue 1")
        tap("Green 2")

        assertEquals("local/score", nav.currentDestination?.route)
        assertEquals("signin", nav.previousBackStackEntry?.destination?.route) // the setup screens are gone
        compose.onNodeWithContentDescription("Point Blue").assertExists()
        assertEquals(LocalMatch(setup, emptyList()), store.saved.single())
    }

    @Test
    fun local_setup_best_of_three_asks_deciding() {
        val store = FakeLocalMatchStore()
        launch(signedOut(), store)
        tap("Local match")
        tap("Advantage")
        tap("Best of three")
        assertEquals("local/deciding", nav.currentDestination?.route)
        tap("Super tie-break")
        tap("Green 1")
        tap("Blue 2")

        assertEquals("local/score", nav.currentDestination?.route)
        val expected = LocalSetup(
            SetRules(DeuceRule.ADVANTAGE, SetsFormat.BEST_OF_3, DecidingSet.SUPER_TIE_BREAK),
            Start(Player(Team.B, 0), 1),
        )
        assertEquals(expected, store.saved.single().setup)
    }

    @Test
    fun local_setup_opens_score_once_the_match_is_saved() {
        val store = FakeLocalMatchStore()
        launch(signedOut(), store)
        tap("Local match")
        tap("Golden point")
        tap("One set")
        tap("Blue 1")
        val gate = CompletableDeferred<Unit>()
        store.gate = gate
        tap("Green 2")
        assertEquals("local/other", nav.currentDestination?.route) // still writing

        compose.runOnUiThread { gate.complete(Unit) }
        compose.waitForIdle()
        assertEquals("local/score", nav.currentDestination?.route)
        assertEquals("signin", nav.previousBackStackEntry?.destination?.route)
    }

    @Test
    fun cold_start_with_stored_match_opens_its_score_then_back_shows_home() {
        launch(FakeContainer(FakeApi(oneMatch), FakeTokens("tok")), stored(listOf(Team.A)))
        assertEquals("local/score", nav.currentDestination?.route)
        compose.onNodeWithText("15").assertExists()

        pressBack()
        assertEquals("matches", nav.currentDestination?.route)
        compose.onNodeWithText("Resume local match").assertExists()
        compose.onNodeWithText("Ana / Rui v João / Marta").assertExists()
    }

    @Test
    fun swipe_back_keeps_the_match_and_entry_resumes_it() {
        val store = FakeLocalMatchStore()
        launch(signedOut(), store)
        tap("Local match")
        tap("Golden point")
        tap("One set")
        tap("Blue 1")
        tap("Green 2")
        compose.onNodeWithContentDescription("Point Blue").performClick()
        compose.waitForIdle()

        pressBack()
        assertEquals("signin", nav.currentDestination?.route)
        assertEquals(listOf(Team.A), storedMatch(store)?.points)
        tap("Resume local match")
        assertEquals("local/score", nav.currentDestination?.route)
        compose.onNodeWithText("15").assertExists()
    }

    @Test
    fun discard_from_entry_after_confirm_removes_it() {
        val store = stored(listOf(Team.A))
        launch(null, store)
        pressBack()
        assertEquals("first-run", nav.currentDestination?.route)

        tap("Discard local match")
        tap("Yes, discard")
        assertNull(storedMatch(store))
        compose.onNodeWithText("Resume local match").assertDoesNotExist()
        compose.onNodeWithText("Local match").assertExists()
    }

    @Test
    fun score_route_pops_when_the_match_is_gone() {
        launch(signedOut(), stored(listOf(Team.A)))
        assertEquals("local/score", nav.currentDestination?.route)

        val model = ViewModelProvider.create(owner)[LocalMatchModel::class]
        compose.runOnUiThread { model.discard() }
        compose.waitForIdle()
        assertEquals("signin", nav.currentDestination?.route)
        assertNull(nav.previousBackStackEntry)
    }

    @Test
    fun finished_back_returns_home_with_local_match_entry() {
        val store = stored(List(24) { Team.A })
        launch(FakeContainer(FakeApi(oneMatch), FakeTokens("tok")), store)
        tap("Back")

        assertEquals("matches", nav.currentDestination?.route)
        assertNull(nav.previousBackStackEntry)
        assertNull(storedMatch(store))
        compose.onNodeWithText("Local match").assertExists()
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
