package io.github.jfms7s.padelscore.local

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.emptyPreferences
import androidx.datastore.preferences.core.stringPreferencesKey
import io.github.jfms7s.padelscore.local.engine.SetRules
import io.github.jfms7s.padelscore.local.engine.Start
import io.github.jfms7s.padelscore.net.Team
import io.github.jfms7s.padelscore.net.WireJson
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.map
import kotlinx.serialization.KSerializer
import kotlinx.serialization.Serializable
import kotlinx.serialization.SerializationException

@Serializable
data class LocalSetup(val rules: SetRules, val start: Start)

/** A local match is its setup and the points won, in order; the score is replayed from them. */
@Serializable
data class LocalMatch(val setup: LocalSetup, val points: List<Team>)

data class LocalStoreState(val match: LocalMatch?, val lastSetup: LocalSetup?)

/** The local match kept on the watch. Writes throw on I/O failure. */
interface LocalMatchStore {
    /** Never throws: an unreadable store, or a value that doesn't decode, reads as null for that field. */
    val state: Flow<LocalStoreState>

    /** Writes the match and records its setup as the last one, in one write. */
    suspend fun save(match: LocalMatch)

    /** Removes the match; the last setup stays. */
    suspend fun clear()
}

/**
 * The DataStore must be one instance per process, built with
 * `ReplaceFileCorruptionHandler { emptyPreferences() }` so a corrupt file reads as empty.
 */
class DataStoreLocalMatchStore(private val dataStore: DataStore<Preferences>) : LocalMatchStore {
    override val state: Flow<LocalStoreState> = dataStore.data
        .catch { emit(emptyPreferences()) }
        .map { LocalStoreState(it.decode(MATCH, LocalMatch.serializer()), it.decode(LAST_SETUP, LocalSetup.serializer())) }

    override suspend fun save(match: LocalMatch) {
        dataStore.edit {
            it[MATCH] = WireJson.encodeToString(LocalMatch.serializer(), match)
            it[LAST_SETUP] = WireJson.encodeToString(LocalSetup.serializer(), match.setup)
        }
    }

    override suspend fun clear() {
        dataStore.edit { it.remove(MATCH) }
    }

    // A value this build can't decode (never written by it) reads as absent, rather than breaking startup.
    private fun <T> Preferences.decode(key: Preferences.Key<String>, serializer: KSerializer<T>): T? =
        this[key]?.let {
            try {
                WireJson.decodeFromString(serializer, it)
            } catch (_: SerializationException) {
                null
            } catch (_: IllegalArgumentException) {
                null
            }
        }

    private companion object {
        val MATCH = stringPreferencesKey("local_match")
        val LAST_SETUP = stringPreferencesKey("last_setup")
    }
}
