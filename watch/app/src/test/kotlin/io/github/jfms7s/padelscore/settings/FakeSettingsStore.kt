package io.github.jfms7s.padelscore.settings

import okhttp3.HttpUrl
import okhttp3.HttpUrl.Companion.toHttpUrl
import java.io.IOException
import java.net.InetAddress

val testSettings = ServerSettings(
    "https://example.test/".toHttpUrl(),
    listOf(InetAddress.getByName("10.0.0.10")),
)

/** In memory, same semantics as DataStoreSettingsStore: no server means nothing is set. */
class FakeSettingsStore(var server: HttpUrl? = null, var fallback: List<InetAddress> = emptyList()) : SettingsStore {
    var failWrites = false
    var writes = 0

    override suspend fun read() = server?.let { ServerSettings(it, fallback) }

    override suspend fun setServer(url: HttpUrl, addresses: List<InetAddress>) = write {
        server = url
        fallback = addresses
    }

    override suspend fun setFallback(addresses: List<InetAddress>) = write { fallback = addresses }

    private fun write(change: () -> Unit) {
        if (failWrites) throw IOException("disk full")
        writes++
        change()
    }
}
