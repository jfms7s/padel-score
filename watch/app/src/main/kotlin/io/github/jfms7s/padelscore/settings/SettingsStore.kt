package io.github.jfms7s.padelscore.settings

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.flow.first
import okhttp3.HttpUrl
import java.net.InetAddress

/** The server settings set on the watch. Writes throw on I/O failure. */
interface SettingsStore {
    /** The settings, or null until a server is set. Never throws: an unreadable store reads as null. */
    suspend fun read(): ServerSettings?

    /** Sets the server and its LAN addresses in one write. */
    suspend fun setServer(url: HttpUrl, addresses: List<InetAddress>)

    suspend fun setFallback(addresses: List<InetAddress>)
}

/**
 * The build has no server: the owner sets it on the watch. The DataStore must be one instance per
 * process, built with `ReplaceFileCorruptionHandler { emptyPreferences() }` so a corrupt file reads
 * as nothing set.
 */
class DataStoreSettingsStore(private val dataStore: DataStore<Preferences>) : SettingsStore {
    override suspend fun read(): ServerSettings? {
        val prefs = try {
            dataStore.data.first()
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            return null
        }
        // A value this build can't parse (never written by it) reads as unset, rather than breaking startup.
        val server = prefs[SERVER]?.let(::parseServerUrl) ?: return null
        return ServerSettings(server, prefs[FALLBACK]?.let(::parseAddressList) ?: emptyList())
    }

    override suspend fun setServer(url: HttpUrl, addresses: List<InetAddress>) {
        dataStore.edit {
            it[SERVER] = originKey(url)
            it[FALLBACK] = addressText(addresses)
        }
    }

    override suspend fun setFallback(addresses: List<InetAddress>) {
        dataStore.edit { it[FALLBACK] = addressText(addresses) }
    }

    private companion object {
        val SERVER = stringPreferencesKey("server_url")
        val FALLBACK = stringPreferencesKey("fallback_addresses")
    }
}
