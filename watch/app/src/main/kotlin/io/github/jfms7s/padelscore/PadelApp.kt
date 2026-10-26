package io.github.jfms7s.padelscore

import android.app.Application
import io.github.jfms7s.padelscore.net.LanFallbackDns
import io.github.jfms7s.padelscore.settings.ServerSettings
import io.github.jfms7s.padelscore.settings.SettingsStore
import kotlinx.coroutines.runBlocking
import okhttp3.HttpUrl.Companion.toHttpUrl

class PadelApp : Application() {
    lateinit var container: AppContainer
        private set

    private lateinit var settingsStore: SettingsStore

    override fun onCreate() {
        super.onCreate()
        val defaults = ServerSettings(
            BuildConfig.SERVER_URL.toHttpUrl(),
            LanFallbackDns.parseAddresses(BuildConfig.SERVER_FALLBACK_IPS),
        )
        settingsStore = settingsStoreOf(this, defaults)
        // One small local file, read once before the first screen needs the server.
        container = runBlocking { build() }
    }

    /** Rebuilds [container] on the stored settings, after the owner changed them on the watch. */
    suspend fun reload(): AppContainer = build().also { container = it }

    private suspend fun build(): AppContainer =
        DefaultAppContainer(this, settingsStore.read(), settingsStore, reloader = ::reload)
}
