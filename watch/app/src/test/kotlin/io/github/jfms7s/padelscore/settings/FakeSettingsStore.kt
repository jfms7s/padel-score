package io.github.jfms7s.padelscore.settings

import okhttp3.HttpUrl
import okhttp3.HttpUrl.Companion.toHttpUrl
import java.io.IOException
import java.net.InetAddress

val testDefaults = ServerSettings(
    "https://example.test/".toHttpUrl(),
    listOf(InetAddress.getByName("192.168.50.10")),
)

/** In memory, same semantics as DataStoreSettingsStore: unset means the default. */
class FakeSettingsStore(override val defaults: ServerSettings = testDefaults) : SettingsStore {
    var server: HttpUrl? = null
    var fallback: List<InetAddress>? = null
    var failWrites = false
    var writes = 0

    override suspend fun read() = ServerSettings(server ?: defaults.serverUrl, fallback ?: defaults.fallback)

    override suspend fun setServer(url: HttpUrl) = write { server = url }

    override suspend fun setFallback(addresses: List<InetAddress>) = write { fallback = addresses }

    override suspend fun reset() = write {
        server = null
        fallback = null
    }

    private fun write(change: () -> Unit) {
        if (failWrites) throw IOException("disk full")
        writes++
        change()
    }
}
