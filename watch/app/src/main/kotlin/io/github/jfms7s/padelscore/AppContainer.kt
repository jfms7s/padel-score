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
import kotlinx.coroutines.flow.Flow
import kotlinx.serialization.builtins.ListSerializer
import okhttp3.HttpUrl
import okhttp3.OkHttpClient
import java.net.InetAddress
import java.time.Duration

/** What the screens need; [WatchApp] takes it so tests can hand in fakes. */
interface AppContainer {
    val serverUrl: HttpUrl
    val api: PadelApi
    val tokens: TokenStore
    val haptics: Haptics
    val deviceName: String

    /** Cold: each collection opens its own connection. */
    fun listStream(): Flow<StreamItem<List<MyMatch>>>

    /** Cold: each collection opens its own connection. */
    fun matchStream(matchId: String): Flow<StreamItem<MatchView>>
}

// DataStore allows one active instance per file, so the delegate keeps it process-wide.
// A corrupt file reads as empty, which means signed out.
private val Context.authDataStore by preferencesDataStore(
    name = "auth",
    corruptionHandler = ReplaceFileCorruptionHandler { emptyPreferences() },
)

/** Manual dependency injection: one instance per process, built by [PadelApp]. */
class DefaultAppContainer(
    context: Context,
    override val serverUrl: HttpUrl,
    serverFallback: List<InetAddress> = emptyList(),
) : AppContainer {
    private val appContext: Context = context.applicationContext

    // The stream client below is derived from this one, so it shares the LAN fallback.
    private val http: OkHttpClient = OkHttpClient.Builder()
        .dns(LanFallbackDns(serverUrl.host, serverFallback))
        .connectTimeout(Duration.ofSeconds(5))
        .callTimeout(Duration.ofSeconds(8))
        .retryOnConnectionFailure(false)
        .build()

    // SSE stays open indefinitely, but the server keep-alives every 10 s, so 20 s of read silence means a dead link.
    private val streamHttp: OkHttpClient = http.newBuilder()
        .callTimeout(Duration.ZERO)
        .readTimeout(Duration.ofSeconds(20))
        .build()

    override val tokens: TokenStore = DataStoreTokenStore(appContext.authDataStore, KeystoreCipher())

    override val api: PadelApi = HttpPadelApi(serverUrl, http, WireJson, token = { tokens.read() })

    override val haptics: Haptics = VibratorHaptics(appContext)

    override val deviceName: String = deviceNameOf(Build.MODEL)

    private val matchListSerializer = ListSerializer(MyMatch.serializer())

    override fun listStream(): Flow<StreamItem<List<MyMatch>>> =
        sseFlow(streamHttp, { api.listStreamRequest() }, "matches", { WireJson.decodeFromString(matchListSerializer, it) })

    override fun matchStream(matchId: String): Flow<StreamItem<MatchView>> =
        sseFlow(streamHttp, { api.matchStreamRequest(matchId) }, "state", { WireJson.decodeFromString(MatchView.serializer(), it) })
}
