package io.github.jfms7s.padelscore.settings

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
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import java.net.InetAddress

@OptIn(ExperimentalCoroutinesApi::class)
class SettingsModelTest {
    private val scheduler = TestCoroutineScheduler()
    private val store = FakeSettingsStore(testSettings.serverUrl, testSettings.fallback)
    private val tokens = MemoryTokenStore().apply { token = "tok" }

    @Before fun setMain() = Dispatchers.setMain(StandardTestDispatcher(scheduler))

    @After fun resetMain() = Dispatchers.resetMain()

    private fun model(current: ServerSettings = testSettings) = SettingsModel(store, current, tokens)

    @Test
    fun new_server_clears_the_token_then_saves_and_applies() {
        val model = model()
        model.submitServer("https://padel.other.test/link")
        scheduler.runCurrent()
        assertNull(tokens.token)
        assertEquals("https://padel.other.test/".toHttpUrl(), store.server)
        assertTrue(model.state.value.applied)
    }

    @Test
    fun same_origin_keeps_the_token_and_needs_no_restart() {
        val model = model()
        model.submitServer("example.test/some/path")
        scheduler.runCurrent()
        assertEquals("tok", tokens.token)
        assertFalse(model.state.value.applied)
        assertNull(model.state.value.error)
    }

    @Test
    fun new_addresses_keep_the_token() {
        val model = model()
        model.submitAddresses("10.0.0.1 10.0.0.2")
        scheduler.runCurrent()
        assertEquals("tok", tokens.token)
        assertEquals(listOf("10.0.0.1", "10.0.0.2"), store.fallback.map { it.hostAddress })
        assertTrue(model.state.value.applied)
    }

    @Test
    fun bad_input_keeps_the_settings_and_shows_an_error() {
        val model = model()
        model.submitServer("http://padel.other.test")
        scheduler.runCurrent()
        assertEquals(SettingsError.BadServer, model.state.value.error)
        model.submitAddresses("10.0.0.1, nope")
        scheduler.runCurrent()
        assertEquals(SettingsError.BadAddresses, model.state.value.error)
        assertEquals(testSettings, model.state.value.settings)
        assertEquals(0, store.writes)
        assertEquals("tok", tokens.token)
        assertFalse(model.state.value.applied)
    }

    @Test
    fun a_good_edit_clears_the_error() {
        val model = model()
        model.submitAddresses("nope")
        model.submitAddresses("")
        scheduler.runCurrent()
        assertNull(model.state.value.error)
        assertEquals(emptyList<InetAddress>(), store.fallback)
    }

    @Test
    fun failed_save_reports_and_does_not_apply() {
        store.failWrites = true
        val model = model()
        model.submitServer("padel.other.test")
        scheduler.runCurrent()
        assertEquals(SettingsError.CouldNotSave, model.state.value.error)
        assertFalse(model.state.value.applied)
        // The token was dropped before the write, so it can never be sent to a server it wasn't issued by.
        assertNull(tokens.token)
    }

    @Test
    fun new_host_drops_the_lan_addresses() {
        val model = model()
        model.submitServer("padel.other.test")
        scheduler.runCurrent()
        assertEquals(emptyList<InetAddress>(), store.fallback)
        // Shown as such on the screen until the app restarts.
        assertEquals(emptyList<InetAddress>(), model.state.value.settings?.fallback)
    }

    @Test
    fun same_host_other_port_keeps_the_addresses_but_not_the_token() {
        val model = model()
        model.submitServer("https://example.test:8443")
        scheduler.runCurrent()
        assertEquals(testSettings.fallback, store.fallback)
        assertEquals(testSettings.fallback, model.state.value.settings?.fallback)
        assertNull(tokens.token)
    }

    @Test
    fun first_server_on_a_new_install_saves_and_applies() {
        val fresh = FakeSettingsStore()
        val model = SettingsModel(fresh, current = null, tokens = null)
        model.submitServer("padel.example.test")
        scheduler.runCurrent()
        assertEquals("https://padel.example.test/".toHttpUrl(), fresh.server)
        assertEquals(emptyList<InetAddress>(), fresh.fallback)
        assertTrue(model.state.value.applied)
    }

    @Test
    fun addresses_before_any_server_are_ignored() {
        val fresh = FakeSettingsStore()
        val model = SettingsModel(fresh, current = null, tokens = null)
        model.submitAddresses("10.0.0.1")
        scheduler.runCurrent()
        assertEquals(0, fresh.writes)
        assertFalse(model.state.value.applied)
    }
}
