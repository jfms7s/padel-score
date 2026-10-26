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
    private val store = FakeSettingsStore()
    private val tokens = MemoryTokenStore().apply { token = "tok" }

    @Before fun setMain() = Dispatchers.setMain(StandardTestDispatcher(scheduler))

    @After fun resetMain() = Dispatchers.resetMain()

    private fun model(current: ServerSettings = testDefaults) = SettingsModel(store, tokens, current)

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
        assertEquals(listOf("10.0.0.1", "10.0.0.2"), store.fallback?.map { it.hostAddress })
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
        assertEquals(testDefaults, model.state.value.settings)
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
    fun reset_back_to_the_default_server_clears_the_token() {
        store.server = "https://padel.other.test/".toHttpUrl()
        val model = model(ServerSettings("https://padel.other.test/".toHttpUrl(), testDefaults.fallback))
        model.reset()
        scheduler.runCurrent()
        assertNull(store.server)
        assertNull(tokens.token)
        assertTrue(model.state.value.applied)
    }

    @Test
    fun reset_of_addresses_only_keeps_the_token() {
        store.fallback = emptyList()
        val model = model(testDefaults.copy(fallback = emptyList()))
        model.reset()
        scheduler.runCurrent()
        assertNull(store.fallback)
        assertEquals("tok", tokens.token)
        assertTrue(model.state.value.applied)
    }

    @Test
    fun new_host_drops_the_lan_addresses() {
        val model = model()
        model.submitServer("padel.other.test")
        scheduler.runCurrent()
        assertEquals(emptyList<InetAddress>(), store.fallback)
        // Shown as such on the screen until the app restarts.
        assertEquals(emptyList<InetAddress>(), model.state.value.settings.fallback)
    }

    @Test
    fun back_to_the_default_host_brings_back_the_default_addresses() {
        store.server = "https://padel.other.test/".toHttpUrl()
        store.fallback = listOf(InetAddress.getByName("10.9.9.9"))
        val model = model(ServerSettings("https://padel.other.test/".toHttpUrl(), store.fallback!!))
        model.submitServer("example.test")
        scheduler.runCurrent()
        assertNull(store.fallback)
        assertEquals(testDefaults, model.state.value.settings)
        assertNull(tokens.token)
    }

    @Test
    fun same_host_other_port_keeps_the_addresses_but_not_the_token() {
        store.fallback = listOf(InetAddress.getByName("10.9.9.9"))
        val model = model(testDefaults.copy(fallback = store.fallback!!))
        model.submitServer("https://example.test:8443")
        scheduler.runCurrent()
        assertEquals(listOf(InetAddress.getByName("10.9.9.9")), store.fallback)
        assertEquals(listOf(InetAddress.getByName("10.9.9.9")), model.state.value.settings.fallback)
        assertNull(tokens.token)
    }
}
