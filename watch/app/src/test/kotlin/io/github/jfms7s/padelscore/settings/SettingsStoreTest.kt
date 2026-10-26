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

    private fun store(defaults: ServerSettings = testDefaults) = DataStoreSettingsStore(
        PreferenceDataStoreFactory.create(
            corruptionHandler = ReplaceFileCorruptionHandler { emptyPreferences() },
            scope = scope,
        ) { file },
        defaults,
    )

    @Test
    fun unset_reads_as_defaults() = runTest {
        assertEquals(testDefaults, store().read())
    }

    @Test
    fun overrides_round_trip() = runTest {
        val store = store()
        store.setServer("https://padel.other.test/link".toHttpUrl())
        store.setFallback(listOf(InetAddress.getByName("10.0.0.7")))
        val read = store.read()
        assertEquals("https://padel.other.test/", read.serverUrl.toString())
        assertEquals(listOf(InetAddress.getByName("10.0.0.7")), read.fallback)
    }

    @Test
    fun empty_addresses_override_means_none() = runTest {
        val store = store()
        store.setFallback(emptyList())
        assertEquals(emptyList<InetAddress>(), store.read().fallback)
        assertEquals(testDefaults.serverUrl, store.read().serverUrl)
    }

    @Test
    fun unset_settings_follow_new_defaults() = runTest {
        store().setFallback(listOf(InetAddress.getByName("10.0.0.7")))
        scope.cancel() // one DataStore per file at a time, as in one process

        // A later build with other defaults: the overridden addresses stay, the server follows the build.
        val newDefaults = ServerSettings("https://new.example.test/".toHttpUrl(), emptyList())
        val freshScope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
        try {
            val later = DataStoreSettingsStore(PreferenceDataStoreFactory.create(scope = freshScope) { file }, newDefaults)
            val read = later.read()
            assertEquals(newDefaults.serverUrl, read.serverUrl)
            assertEquals(listOf(InetAddress.getByName("10.0.0.7")), read.fallback)
        } finally {
            freshScope.cancel()
        }
    }

    @Test
    fun reset_drops_the_overrides() = runTest {
        val store = store()
        store.setServer("https://padel.other.test/".toHttpUrl())
        store.setFallback(emptyList())
        store.reset()
        assertEquals(testDefaults, store.read())
    }

    @Test
    fun corrupt_file_reads_as_defaults_and_accepts_new_overrides() = runTest {
        file.writeBytes(byteArrayOf(0x0F, 0x7A, 0x01, 0x55, 0x33))
        val store = store()
        assertEquals(testDefaults, store.read())
        store.setServer("https://padel.other.test/".toHttpUrl())
        assertEquals("https://padel.other.test/", store.read().serverUrl.toString())
    }
}
