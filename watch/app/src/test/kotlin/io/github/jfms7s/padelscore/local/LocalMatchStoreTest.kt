package io.github.jfms7s.padelscore.local

import androidx.datastore.core.DataStore
import androidx.datastore.core.handlers.ReplaceFileCorruptionHandler
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.emptyPreferences
import androidx.datastore.preferences.core.stringPreferencesKey
import io.github.jfms7s.padelscore.local.engine.DecidingSet
import io.github.jfms7s.padelscore.local.engine.DeuceRule
import io.github.jfms7s.padelscore.local.engine.SetRules
import io.github.jfms7s.padelscore.local.engine.SetsFormat
import io.github.jfms7s.padelscore.local.engine.Start
import io.github.jfms7s.padelscore.net.Player
import io.github.jfms7s.padelscore.net.Team
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.runTest
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Before
import org.junit.Test
import java.io.File
import java.nio.file.Files

class LocalMatchStoreTest {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
    private lateinit var dir: File
    private lateinit var file: File
    private lateinit var dataStore: DataStore<Preferences>

    private val setup = LocalSetup(
        SetRules(DeuceRule.GOLDEN_POINT, SetsFormat.BEST_OF_3, DecidingSet.SUPER_TIE_BREAK),
        Start(Player(Team.B, 1), 0),
    )
    private val match = LocalMatch(setup, listOf(Team.A, Team.B, Team.A))

    @Before
    fun setUp() {
        dir = Files.createTempDirectory("local-match").toFile()
        file = File(dir, "local_match.preferences_pb")
    }

    @After
    fun tearDown() {
        scope.cancel()
        dir.deleteRecursively()
    }

    private fun store(): DataStoreLocalMatchStore {
        dataStore = PreferenceDataStoreFactory.create(
            corruptionHandler = ReplaceFileCorruptionHandler { emptyPreferences() },
            scope = scope,
        ) { file }
        return DataStoreLocalMatchStore(dataStore)
    }

    @Test
    fun empty_store_has_no_match_and_no_last_setup() = runTest {
        assertEquals(LocalStoreState(null, null), store().state.first())
    }

    @Test
    fun save_then_read_round_trips_and_records_last_setup() = runTest {
        val store = store()
        store.save(match)
        assertEquals(LocalStoreState(match, setup), store.state.first())
    }

    @Test
    fun clear_keeps_last_setup() = runTest {
        val store = store()
        store.save(match)
        store.clear()
        assertEquals(LocalStoreState(null, setup), store.state.first())
    }

    @Test
    fun garbage_match_reads_as_none() = runTest {
        val store = store()
        store.save(match)
        dataStore.edit { it[stringPreferencesKey("local_match")] = "{nope" }
        val state = store.state.first()
        assertNull(state.match)
        assertEquals(setup, state.lastSetup)
    }

    @Test
    fun corrupt_file_reads_as_empty() = runTest {
        file.writeBytes(byteArrayOf(0x0F, 0x7A, 0x01, 0x55, 0x33))
        val store = store()
        assertEquals(LocalStoreState(null, null), store.state.first())
        store.save(match)
        assertEquals(match, store.state.first().match)
    }
}
