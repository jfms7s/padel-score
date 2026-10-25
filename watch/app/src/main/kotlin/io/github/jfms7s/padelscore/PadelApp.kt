package io.github.jfms7s.padelscore

import android.app.Application
import okhttp3.HttpUrl.Companion.toHttpUrl

class PadelApp : Application() {
    lateinit var container: AppContainer
        private set

    override fun onCreate() {
        super.onCreate()
        container = AppContainer(this, BuildConfig.SERVER_URL.toHttpUrl())
    }
}
