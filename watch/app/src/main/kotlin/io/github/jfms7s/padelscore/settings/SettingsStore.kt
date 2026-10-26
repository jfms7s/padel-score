package io.github.jfms7s.padelscore.settings

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.flow.first
import okhttp3.HttpUrl
import java.net.InetAddress

/** The owner's overrides of the build's server settings. Writes throw on I/O failure. */
interface SettingsStore {
    /** The build's values, used for every setting the owner has not overridden. */
    val defaults: ServerSettings

    /** The overrides on top of [defaults]. Never throws: an unreadable store reads as the defaults. */
    suspend fun read(): ServerSettings

    suspend fun setServer(url: HttpUrl)

    suspend fun setFallback(addresses: List<InetAddress>)

    /** Drops every override, so the defaults (including a later build's) apply again. */
    suspend fun reset()
}

/**
 * Each setting is its own key, and an absent key means "use the default", so a new build's defaults
 * apply to whatever the owner never changed. The DataStore must be one instance per process, built
 * with `ReplaceFileCorruptionHandler { emptyPreferences() }` so a corrupt file reads as the defaults.
 */
class DataStoreSettingsStore(
    private val dataStore: DataStore<Preferences>,
    override val defaults: ServerSettings,
) : SettingsStore {
    override suspend fun read(): ServerSettings {
        val prefs = try {
            dataStore.data.first()
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            return defaults
        }
        // A value this build can't parse (never written by it) falls back too, rather than breaking startup.
        return ServerSettings(
            serverUrl = prefs[SERVER]?.let(::parseServerUrl) ?: defaults.serverUrl,
            fallback = prefs[FALLBACK]?.let(::parseAddressList) ?: defaults.fallback,
        )
    }

    override suspend fun setServer(url: HttpUrl) {
        dataStore.edit { it[SERVER] = originOf(url).toString() }
    }

    override suspend fun setFallback(addresses: List<InetAddress>) {
        dataStore.edit { it[FALLBACK] = addressText(addresses) }
    }

    override suspend fun reset() {
        dataStore.edit {
            it.remove(SERVER)
            it.remove(FALLBACK)
        }
    }

    private companion object {
        val SERVER = stringPreferencesKey("server_url")
        val FALLBACK = stringPreferencesKey("fallback_addresses")
    }
}
