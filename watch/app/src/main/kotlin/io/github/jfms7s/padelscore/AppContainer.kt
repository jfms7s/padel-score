package io.github.jfms7s.padelscore

import android.content.Context
import android.os.Build
import androidx.datastore.core.handlers.ReplaceFileCorruptionHandler
import androidx.datastore.preferences.core.emptyPreferences
import androidx.datastore.preferences.preferencesDataStore
import io.github.jfms7s.padelscore.auth.DataStoreTokenStore
import io.github.jfms7s.padelscore.auth.KeystoreCipher
import io.github.jfms7s.padelscore.auth.TokenStore
import io.github.jfms7s.padelscore.auth.deviceNameOf
import io.github.jfms7s.padelscore.haptics.Haptics
import io.github.jfms7s.padelscore.haptics.VibratorHaptics
import io.github.jfms7s.padelscore.net.HttpPadelApi
import io.github.jfms7s.padelscore.net.LanFallbackDns
import io.github.jfms7s.padelscore.net.MatchView
import io.github.jfms7s.padelscore.net.MyMatch
import io.github.jfms7s.padelscore.net.PadelApi
import io.github.jfms7s.padelscore.net.StreamItem
import io.github.jfms7s.padelscore.net.WireJson
import io.github.jfms7s.padelscore.net.sseFlow
import io.github.jfms7s.padelscore.settings.DataStoreSettingsStore
import io.github.jfms7s.padelscore.settings.ServerSettings
import io.github.jfms7s.padelscore.settings.SettingsStore
import io.github.jfms7s.padelscore.settings.originKey
import kotlinx.coroutines.flow.Flow
import kotlinx.serialization.builtins.ListSerializer
import okhttp3.HttpUrl
import okhttp3.OkHttpClient
import java.time.Duration

/** What the screens need; [WatchApp] takes it so tests can hand in fakes. */
interface AppContainer {
    /** The settings this container was built with. */
    val settings: ServerSettings
    val serverUrl: HttpUrl get() = settings.serverUrl
    val settingsStore: SettingsStore
    val api: PadelApi
    val tokens: TokenStore
    val haptics: Haptics
    val deviceName: String

    /** Cold: each collection opens its own connection. */
    fun listStream(): Flow<StreamItem<List<MyMatch>>>

    /** Cold: each collection opens its own connection. */
    fun matchStream(matchId: String): Flow<StreamItem<MatchView>>
}

/** Holds the process's [AppContainer]; the [MainActivity] asks it for a new one after the settings change. */
interface ContainerHost {
    val container: AppContainer

    /** Replaces [container] with one built on the stored settings. */
    suspend fun reload(): AppContainer
}

// DataStore allows one active instance per file, so the delegate keeps it process-wide.
// A corrupt file reads as empty, which means signed out.
private val Context.authDataStore by preferencesDataStore(
    name = "auth",
    corruptionHandler = ReplaceFileCorruptionHandler { emptyPreferences() },
)

private val Context.settingsDataStore by preferencesDataStore(
    name = "server_settings",
    corruptionHandler = ReplaceFileCorruptionHandler { emptyPreferences() },
)

/** The process-wide store of the owner's server settings, on top of the build's [defaults]. */
fun settingsStoreOf(context: Context, defaults: ServerSettings): SettingsStore =
    DataStoreSettingsStore(context.applicationContext.settingsDataStore, defaults)

/**
 * Manual dependency injection, built by [PadelApp]: one instance per process until the server
 * settings change, then [PadelApp.reload] replaces it.
 */
class DefaultAppContainer(
    context: Context,
    override val settings: ServerSettings,
    override val settingsStore: SettingsStore,
) : AppContainer {
    private val appContext: Context = context.applicationContext

    // The stream client below is derived from this one, so it shares the LAN fallback.
    private val http: OkHttpClient = OkHttpClient.Builder()
        .dns(LanFallbackDns(settings.serverUrl.host, settings.fallback))
        .connectTimeout(Duration.ofSeconds(5))
        .callTimeout(Duration.ofSeconds(8))
        .retryOnConnectionFailure(false)
        .build()

    // SSE stays open indefinitely, but the server keep-alives every 10 s, so 20 s of read silence means a dead link.
    private val streamHttp: OkHttpClient = http.newBuilder()
        .callTimeout(Duration.ZERO)
        .readTimeout(Duration.ofSeconds(20))
        .build()

    override val tokens: TokenStore = DataStoreTokenStore(
        appContext.authDataStore,
        KeystoreCipher(),
        origin = originKey(settings.serverUrl),
        legacyOrigin = originKey(settingsStore.defaults.serverUrl),
    )

    override val api: PadelApi = HttpPadelApi(serverUrl, http, WireJson, token = { tokens.read() })

    override val haptics: Haptics = VibratorHaptics(appContext)

    override val deviceName: String = deviceNameOf(Build.MODEL)

    private val matchListSerializer = ListSerializer(MyMatch.serializer())

    override fun listStream(): Flow<StreamItem<List<MyMatch>>> =
        sseFlow(streamHttp, { api.listStreamRequest() }, "matches", { WireJson.decodeFromString(matchListSerializer, it) })

    override fun matchStream(matchId: String): Flow<StreamItem<MatchView>> =
        sseFlow(streamHttp, { api.matchStreamRequest(matchId) }, "state", { WireJson.decodeFromString(MatchView.serializer(), it) })
}
