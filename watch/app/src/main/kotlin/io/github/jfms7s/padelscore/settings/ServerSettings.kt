package io.github.jfms7s.padelscore.settings

import io.github.jfms7s.padelscore.net.parseIpLiteral
import okhttp3.HttpUrl
import okhttp3.HttpUrl.Companion.toHttpUrlOrNull
import java.net.InetAddress

/** Where the watch talks to: the server's origin, and the LAN addresses tried when DNS has no LAN answer. */
data class ServerSettings(val serverUrl: HttpUrl, val fallback: List<InetAddress>)

/**
 * The hosts the manifest's network security config lets the app reach over cleartext (the emulator's
 * alias for the dev machine, and localhost). Any other http URL would fail at runtime, so it is refused.
 */
private val cleartextHosts = setOf("10.0.2.2", "localhost")

/**
 * Parses what the owner typed or said as the server URL, reduced to its origin (`scheme://host[:port]/`).
 * Without a scheme, https is assumed. Returns null unless it is an https URL, or http to a dev host.
 */
fun parseServerUrl(text: String): HttpUrl? {
    val trimmed = text.trim()
    if (trimmed.isEmpty()) return null
    val withScheme = if ("://" in trimmed) trimmed else "https://$trimmed"
    val url = withScheme.toHttpUrlOrNull() ?: return null
    if (url.scheme == "http" && url.host !in cleartextHosts) return null
    return originOf(url)
}

/** [url] with only scheme, host and port left. */
fun originOf(url: HttpUrl): HttpUrl =
    url.newBuilder().username("").password("").encodedPath("/").query(null).fragment(null).build()

/** The origin as a string, which is what a stored token is bound to. */
fun originKey(url: HttpUrl): String = originOf(url).toString()

fun sameOrigin(a: HttpUrl, b: HttpUrl): Boolean = a.scheme == b.scheme && a.host == b.host && a.port == b.port

/**
 * Parses comma- or space-separated IP literals. Empty input means no addresses. Returns null if any
 * entry is not an IP literal, so a typo never silently drops an address. Never does a DNS lookup.
 */
fun parseAddressList(text: String): List<InetAddress>? {
    val tokens = text.split(',', ' ', '\n', '\t').map(String::trim).filter(String::isNotEmpty)
    val parsed = tokens.map { parseIpLiteral(it) ?: return null }
    return parsed.distinct()
}

/** The server URL as shown on the watch: no scheme, no trailing slash. */
fun displayUrl(url: HttpUrl): String =
    url.toString().removePrefix("https://").removePrefix("http://").removeSuffix("/")

/** The addresses as stored and shown: comma-separated, empty for none. */
fun addressText(addresses: List<InetAddress>): String = addresses.joinToString(", ") { it.hostAddress.orEmpty() }
