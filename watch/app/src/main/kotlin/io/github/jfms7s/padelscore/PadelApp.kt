package io.github.jfms7s.padelscore

import android.app.Application
import io.github.jfms7s.padelscore.net.LanFallbackDns
import okhttp3.HttpUrl.Companion.toHttpUrl

class PadelApp : Application() {
    lateinit var container: AppContainer
        private set

    override fun onCreate() {
        super.onCreate()
        container = DefaultAppContainer(
            this,
            BuildConfig.SERVER_URL.toHttpUrl(),
            LanFallbackDns.parseAddresses(BuildConfig.SERVER_FALLBACK_IPS),
        )
    }
}
