package io.github.jfms7s.padelscore

import android.app.Activity
import android.app.Application
import android.app.RemoteInput
import android.content.Intent
import android.os.Bundle
import androidx.compose.ui.test.junit4.v2.createEmptyComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.lifecycle.Lifecycle
import androidx.test.core.app.ActivityScenario
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.wear.input.RemoteInputIntentHelper
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
import io.github.jfms7s.padelscore.settings.FakeSettingsStore
import io.github.jfms7s.padelscore.settings.ServerSettings
import io.github.jfms7s.padelscore.settings.testSettings
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.emptyFlow
import kotlinx.coroutines.flow.map
import okhttp3.Request
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/** Stands in for [PadelApp] so the real [MainActivity] runs on fake containers. */
class TestApp : Application(), ContainerHost {
    override val settingsStore = FakeSettingsStore()
    val tokens = MemoryTokens()
    override var container: AppContainer? = null
    val built = mutableListOf<SignInContainer>()

    /** Starts the app as if the owner had set [settings] on an earlier run. */
    fun configure(settings: ServerSettings) {
        settingsStore.server = settings.serverUrl
        settingsStore.fallback = settings.fallback
        container = SignInContainer(settings, settingsStore, tokens)
    }

    override suspend fun reload(): AppContainer? =
        settingsStore.read()?.let { settings ->
            SignInContainer(settings, settingsStore, tokens).also { built += it }
        }.also { container = it }
}

class MemoryTokens : TokenStore {
    private val stored = MutableStateFlow<String?>(null)
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

/** Sign-in waits on the server forever, and notes when that wait is cancelled. */
class HangingApi : PadelApi {
    var started = 0
    var cancelled = 0

    override suspend fun startDevice(deviceName: String): ApiResult<DeviceStart> {
        started++
        try {
            awaitCancellation()
        } finally {
            cancelled++
        }
    }

    override suspend fun pollToken(deviceCode: String): TokenPoll = TokenPoll.Pending
    override suspend fun myMatches(): ApiResult<List<MyMatch>> = ApiResult.Ok(emptyList())
    override suspend fun send(matchId: String, request: CommandRequest): SendResult = SendResult.NotSent("unused")
    override suspend fun listStreamRequest(): Request = error("unused")
    override suspend fun matchStreamRequest(matchId: String): Request = error("unused")
}

class SignInContainer(
    override val settings: ServerSettings,
    override val settingsStore: FakeSettingsStore,
    override val tokens: MemoryTokens,
) : AppContainer {
    override val api = HangingApi()
    override val haptics: Haptics = FakeHaptics()
    override val deviceName = "Test watch"
    override fun listStream(): Flow<StreamItem<List<MyMatch>>> = emptyFlow()
    override fun matchStream(matchId: String): Flow<StreamItem<MatchView>> = emptyFlow()
}

@RunWith(AndroidJUnit4::class)
@Config(application = TestApp::class, qualifiers = "w227dp-h227dp-round-watch")
class MainActivityTest {
    @get:Rule val compose = createEmptyComposeRule()

    private fun answerServerInput(activity: MainActivity, text: String) {
        compose.onNodeWithText("Server").performClick()
        val shadow = shadowOf(activity)
        val input = checkNotNull(shadow.nextStartedActivityForResult)
        val inputs = checkNotNull(RemoteInputIntentHelper.getRemoteInputsExtra(input.intent))
        val data = Intent()
        RemoteInput.addResultsToIntent(
            inputs.toTypedArray(),
            data,
            Bundle().apply { putCharSequence(inputs.single().resultKey, text) },
        )
        shadow.receiveResult(input.intent, Activity.RESULT_OK, data)
        compose.waitForIdle()
    }

    @Test
    fun first_run_opens_on_the_settings_then_restarts_on_the_new_server() {
        val scenario = ActivityScenario.launch(MainActivity::class.java)
        lateinit var activity: MainActivity
        scenario.onActivity { activity = it }
        val app = activity.application as TestApp
        compose.waitForIdle()
        compose.onNodeWithText("Not set").assertExists()

        answerServerInput(activity, "padel.example.test")

        assertTrue(activity.isFinishing)
        assertEquals(MainActivity::class.java.name, shadowOf(activity).nextStartedActivity.component?.className)
        assertEquals("https://padel.example.test/", app.container?.serverUrl.toString())
        scenario.moveToState(Lifecycle.State.DESTROYED)
    }

    @Test
    fun new_server_restarts_the_activity_and_cancels_the_old_screens() {
        ApplicationProvider.getApplicationContext<TestApp>().configure(testSettings)
        val scenario = ActivityScenario.launch(MainActivity::class.java)
        lateinit var activity: MainActivity
        scenario.onActivity { activity = it }
        val app = activity.application as TestApp
        val old = app.container as SignInContainer
        compose.waitForIdle()
        assertEquals(1, old.api.started) // the sign-in screen is waiting on the old server

        compose.onNodeWithText("Server settings").performClick()
        answerServerInput(activity, "padel.other.test")
        val shadow = shadowOf(activity)

        // A fresh activity on a container for the new server replaces this one.
        assertTrue(activity.isFinishing)
        assertEquals(MainActivity::class.java.name, shadow.nextStartedActivity.component?.className)
        assertEquals("https://padel.other.test/", app.container?.serverUrl.toString())
        assertEquals(1, app.built.size)

        assertEquals(0, old.api.cancelled)

        // The system then destroys the finished activity (Robolectric leaves that step to the test).
        // Not a configuration change, so its models are cleared: the old server's sign-in stops, and
        // can't finish later and leave that server's token behind.
        assertFalse(activity.isChangingConfigurations)
        scenario.moveToState(Lifecycle.State.DESTROYED)
        assertEquals(1, old.api.cancelled)
        assertEquals(1, app.tokens.clears)
    }
}
