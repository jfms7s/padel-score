package io.github.jfms7s.padelscore.settings

import androidx.datastore.core.handlers.ReplaceFileCorruptionHandler
import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import androidx.datastore.preferences.core.emptyPreferences
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.test.runTest
import okhttp3.HttpUrl.Companion.toHttpUrl
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Before
import org.junit.Test
import java.io.File
import java.net.InetAddress
import java.nio.file.Files

class SettingsStoreTest {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
    private lateinit var dir: File
    private lateinit var file: File

    @Before
    fun setUp() {
        dir = Files.createTempDirectory("settings").toFile()
        file = File(dir, "server_settings.preferences_pb")
    }

    @After
    fun tearDown() {
        scope.cancel()
        dir.deleteRecursively()
    }

    private fun store() = DataStoreSettingsStore(
        PreferenceDataStoreFactory.create(
            corruptionHandler = ReplaceFileCorruptionHandler { emptyPreferences() },
            scope = scope,
        ) { file },
    )

    @Test
    fun nothing_set_reads_as_null() = runTest {
        assertNull(store().read())
    }

    @Test
    fun settings_round_trip() = runTest {
        val store = store()
        store.setServer("https://padel.other.test/link".toHttpUrl(), emptyList())
        store.setFallback(listOf(InetAddress.getByName("10.0.0.7")))
        val read = checkNotNull(store.read())
        assertEquals("https://padel.other.test/", read.serverUrl.toString())
        assertEquals(listOf(InetAddress.getByName("10.0.0.7")), read.fallback)
    }

    @Test
    fun addresses_without_a_server_still_read_as_null() = runTest {
        val store = store()
        store.setFallback(listOf(InetAddress.getByName("10.0.0.7")))
        assertNull(store.read())
    }

    @Test
    fun server_change_sets_the_addresses_in_the_same_write() = runTest {
        val store = store()
        store.setServer("https://example.test/".toHttpUrl(), listOf(InetAddress.getByName("10.0.0.7")))
        assertEquals(testSettings.copy(fallback = listOf(InetAddress.getByName("10.0.0.7"))), store.read())
        store.setServer("https://padel.other.test/".toHttpUrl(), emptyList())
        assertEquals(emptyList<InetAddress>(), store.read()?.fallback)
    }

    @Test
    fun corrupt_file_reads_as_null_and_accepts_new_settings() = runTest {
        file.writeBytes(byteArrayOf(0x0F, 0x7A, 0x01, 0x55, 0x33))
        val store = store()
        assertNull(store.read())
        store.setServer("https://padel.other.test/".toHttpUrl(), emptyList())
        assertEquals("https://padel.other.test/", store.read()?.serverUrl.toString())
    }
}
