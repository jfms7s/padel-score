package io.github.jfms7s.padelscore

import android.app.Application
import io.github.jfms7s.padelscore.haptics.VibratorHaptics
import io.github.jfms7s.padelscore.local.DataStoreLocalMatchStore
import io.github.jfms7s.padelscore.local.LocalDeps
import io.github.jfms7s.padelscore.settings.SettingsStore
import kotlinx.coroutines.runBlocking

class PadelApp : Application(), ContainerHost {
    override var container: AppContainer? = null
        private set

    override lateinit var settingsStore: SettingsStore
        private set

    override lateinit var local: LocalDeps
        private set

    override fun onCreate() {
        super.onCreate()
        settingsStore = settingsStoreOf(this)
        local = LocalDeps(DataStoreLocalMatchStore(localMatchDataStore), VibratorHaptics(this))
        // One small local file, read once before the first screen needs the server.
        container = runBlocking { build() }
    }

    /** Rebuilds [container] on the stored settings, after the owner changed them on the watch. */
    override suspend fun reload(): AppContainer? = build().also { container = it }

    private suspend fun build(): AppContainer? =
        settingsStore.read()?.let { DefaultAppContainer(this, it, settingsStore) }
}
