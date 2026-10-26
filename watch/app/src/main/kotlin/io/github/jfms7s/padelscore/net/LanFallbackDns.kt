package io.github.jfms7s.padelscore.net

import okhttp3.Dns
import java.net.Inet6Address
import java.net.InetAddress
import java.net.UnknownHostException
import java.util.concurrent.ExecutionException
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.TimeoutException

/**
 * Resolves the server's host so the watch reaches a LAN-only server on any network path.
 *
 * Over the phone's Bluetooth proxy the watch gets DNS servers that know only the public answer
 * (or don't answer at all), while the LAN addresses themselves are reachable. So for [host]: if the
 * system lookup returns a private address it is used as is; if it fails, hangs past [timeoutMillis]
 * or returns only public addresses, the configured [fallback] addresses go first (any public answer
 * is kept after them). TLS still verifies the certificate against the URL's host name, so this
 * changes where the watch connects, not whom it trusts. Every other host goes to [system] untouched.
 */
class LanFallbackDns(
    private val host: String,
    private val fallback: List<InetAddress>,
    private val system: Dns = Dns.SYSTEM,
    private val timeoutMillis: Long = 3_000,
    private val executor: ExecutorService = lookupExecutor,
) : Dns {
    override fun lookup(hostname: String): List<InetAddress> {
        if (!hostname.equals(host, ignoreCase = true) || fallback.isEmpty()) return system.lookup(hostname)
        val answer = try {
            // A blocking lookup can't be interrupted, so it runs on its own thread and is abandoned on timeout.
            executor.submit<List<InetAddress>> { system.lookup(hostname) }.get(timeoutMillis, TimeUnit.MILLISECONDS)
        } catch (_: TimeoutException) {
            emptyList()
        } catch (_: ExecutionException) {
            emptyList()
        } catch (_: InterruptedException) {
            // Not an IOException, so it would escape the callers' error handling; keep the flag and fall back.
            Thread.currentThread().interrupt()
            emptyList()
        }
        if (answer.any { it.isPrivate() }) return answer
        return fallback + answer.filterNot { it in fallback }
    }

    companion object {
        private val lookupExecutor: ExecutorService = Executors.newCachedThreadPool { runnable ->
            Thread(runnable, "padel-dns").apply { isDaemon = true }
        }

        /**
         * Parses a comma-separated list of IP literals (from the `padelServerFallbackIps` Gradle
         * property). Anything that isn't an IP literal is skipped, so this never does a DNS lookup.
         */
        fun parseAddresses(csv: String): List<InetAddress> =
            csv.split(',').map(String::trim).filter(::isIpLiteral).mapNotNull {
                try {
                    InetAddress.getByName(it)
                } catch (_: UnknownHostException) {
                    null
                }
            }

        private val ipv4 = Regex("""\d{1,3}(\.\d{1,3}){3}""")

        private fun isIpLiteral(s: String): Boolean =
            ipv4.matches(s) && s.split('.').all { it.toInt() <= 255 } ||
                s.contains(':') && s.all { it.isDigit() || it in 'a'..'f' || it in 'A'..'F' || it == ':' || it == '.' }
    }
}

/** RFC 1918 / link-local / loopback, plus IPv6 unique-local (fc00::/7). */
internal fun InetAddress.isPrivate(): Boolean =
    isSiteLocalAddress || isLinkLocalAddress || isLoopbackAddress ||
        (this is Inet6Address && (address[0].toInt() and 0xFE) == 0xFC)
