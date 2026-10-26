package io.github.jfms7s.padelscore.settings

import android.app.Activity
import android.app.RemoteInput
import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.compose.ui.test.assertHeightIsAtLeast
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
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
import org.junit.After
import org.junit.Assert.assertEquals
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
    private val store = FakeSettingsStore(testSettings.serverUrl, testSettings.fallback)

    @Before fun setMain() = Dispatchers.setMain(StandardTestDispatcher(scheduler))

    @After fun resetMain() = Dispatchers.resetMain()

    private fun show(settings: ServerSettings? = testSettings): SettingsModel {
        val model = SettingsModel(store, settings, MemoryTokenStore())
        compose.setContent { SettingsScreen(model, onApplied = {}) }
        return model
    }

    @Test
    fun rows_show_the_current_values() {
        show()
        compose.onNodeWithText("Server").assertHeightIsAtLeast(48.dp)
        compose.onNodeWithText("example.test").assertExists()
        compose.onNodeWithText("LAN addresses").assertHeightIsAtLeast(48.dp)
        compose.onNodeWithText("10.0.0.10").assertExists()
    }

    @Test
    fun no_addresses_shows_none() {
        show(testSettings.copy(fallback = emptyList()))
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
        assertEquals(testSettings.serverUrl, store.server)
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
        assertEquals(0, store.writes)
    }

    @Test
    fun bad_addresses_show_the_error_and_keep_the_old_ones() {
        val model = show()
        compose.runOnUiThread { model.submitAddresses("10.0.0.20, nope") }
        compose.onNodeWithText("Not valid. Use IP addresses separated by commas.").assertExists()
        compose.onNodeWithText("10.0.0.10").assertExists()
        assertEquals(0, store.writes)
    }

    @Test
    fun first_run_asks_for_the_server_and_hides_the_addresses() {
        show(null)
        compose.onNodeWithText("Not set").assertExists()
        compose.onNodeWithText("LAN addresses").assertDoesNotExist()
        answerInput("Server", "padel.example.test")
        assertEquals("https://padel.example.test/", store.server.toString())
    }

    @Test
    fun there_is_no_reset() {
        show()
        compose.onNodeWithText("Reset", substring = true).assertDoesNotExist()
    }
}
