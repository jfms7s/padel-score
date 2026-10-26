package io.github.jfms7s.padelscore.settings

import android.app.Activity
import android.app.RemoteInput
import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.compose.ui.test.assertHeightIsAtLeast
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.hasScrollToNodeAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.wear.input.RemoteInputIntentHelper
import io.github.jfms7s.padelscore.auth.MemoryTokenStore
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestCoroutineScheduler
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.setMain
import okhttp3.HttpUrl.Companion.toHttpUrl
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(AndroidJUnit4::class)
@Config(qualifiers = "w227dp-h227dp-round-watch")
class SettingsScreenTest {
    @get:Rule val compose = createAndroidComposeRule<ComponentActivity>()

    private val scheduler = TestCoroutineScheduler()
    private val store = FakeSettingsStore()

    @Before fun setMain() = Dispatchers.setMain(StandardTestDispatcher(scheduler))

    @After fun resetMain() = Dispatchers.resetMain()

    private fun show(settings: ServerSettings = testDefaults): SettingsModel {
        val model = SettingsModel(store, MemoryTokenStore(), settings)
        compose.setContent { SettingsScreen(model, onApplied = {}) }
        return model
    }

    @Test
    fun rows_show_the_current_values() {
        show()
        compose.onNodeWithText("Server").assertHeightIsAtLeast(48.dp)
        compose.onNodeWithText("example.test").assertExists()
        compose.onNodeWithText("LAN addresses").assertHeightIsAtLeast(48.dp)
        compose.onNodeWithText("192.168.50.10").assertExists()
    }

    @Test
    fun no_addresses_shows_none() {
        show(testDefaults.copy(fallback = emptyList()))
        compose.onNodeWithText("None").assertExists()
    }

    /** Taps [row], checks the system input was asked for, and answers it with [text]. */
    private fun answerInput(row: String, text: String) {
        compose.onNodeWithText(row).performClick()
        val shadow = shadowOf(compose.activity)
        val started = checkNotNull(shadow.nextStartedActivityForResult) { "no input opened" }
        assertTrue(RemoteInputIntentHelper.isActionRemoteInput(started.intent))
        val inputs = checkNotNull(RemoteInputIntentHelper.getRemoteInputsExtra(started.intent))
        val data = Intent()
        RemoteInput.addResultsToIntent(
            inputs.toTypedArray(),
            data,
            Bundle().apply { putCharSequence(inputs.single().resultKey, text) },
        )
        shadow.receiveResult(started.intent, Activity.RESULT_OK, data)
        compose.waitForIdle()
        scheduler.runCurrent()
        compose.waitForIdle()
    }

    @Test
    fun server_row_takes_system_input_and_validates_it() {
        show()
        answerInput("Server", "http://padel.other.test")
        compose.onNodeWithText("Not a valid server. Use an https address.").assertExists()
        compose.onNodeWithText("example.test").assertExists()
        assertNull(store.server)
    }

    @Test
    fun addresses_row_takes_system_input_and_saves_it() {
        show()
        answerInput("LAN addresses", "10.0.0.1, 10.0.0.2")
        assertEquals(listOf("10.0.0.1", "10.0.0.2"), store.fallback?.map { it.hostAddress })
        compose.onNodeWithText("10.0.0.1, 10.0.0.2").assertExists()
    }

    @Test
    fun cancelled_input_changes_nothing() {
        show()
        compose.onNodeWithText("Server").performClick()
        val shadow = shadowOf(compose.activity)
        val started = checkNotNull(shadow.nextStartedActivityForResult)
        shadow.receiveResult(started.intent, Activity.RESULT_CANCELED, null)
        compose.waitForIdle()
        scheduler.runCurrent()
        assertEquals(0, store.writes)
        compose.onNodeWithText("example.test").assertExists()
    }

    @Test
    fun bad_server_shows_the_error_and_keeps_the_old_one() {
        val model = show()
        compose.runOnUiThread { model.submitServer("http://padel.other.test") }
        compose.onNodeWithText("Not a valid server. Use an https address.").assertExists()
        compose.onNodeWithText("example.test").assertExists()
        assertNull(store.server)
    }

    @Test
    fun bad_addresses_show_the_error_and_keep_the_old_ones() {
        val model = show()
        compose.runOnUiThread { model.submitAddresses("192.168.50.20, nope") }
        compose.onNodeWithText("Not valid. Use IP addresses separated by commas.").assertExists()
        compose.onNodeWithText("192.168.50.10").assertExists()
        assertNull(store.fallback)
    }

    @Test
    fun reset_needs_confirm() {
        store.server = "https://padel.other.test/".toHttpUrl()
        show(testDefaults.copy(serverUrl = "https://padel.other.test/".toHttpUrl()))
        scrollTo("Reset to defaults")
        compose.onNodeWithText("Reset to defaults").assertHeightIsAtLeast(48.dp).performClick()
        compose.waitForIdle()
        scheduler.runCurrent()
        assertEquals(0, store.writes)
        compose.onNodeWithText("Uses the server and LAN addresses", substring = true).assertExists()
        compose.onNodeWithText("Yes, reset").performClick()
        compose.waitForIdle()
        scheduler.runCurrent()
        assertEquals(1, store.writes)
        assertNull(store.server)
    }

    @Test
    fun reset_cancel_changes_nothing() {
        show()
        scrollTo("Reset to defaults")
        compose.onNodeWithText("Reset to defaults").performClick()
        compose.onNodeWithText("Cancel").performClick()
        compose.waitForIdle()
        scheduler.runCurrent()
        assertEquals(0, store.writes)
    }

    private fun scrollTo(text: String) {
        compose.onNode(hasScrollToNodeAction()).performScrollToNode(hasText(text))
    }
}
