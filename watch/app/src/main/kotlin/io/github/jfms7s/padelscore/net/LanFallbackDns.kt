package io.github.jfms7s.padelscore.net

import okhttp3.Dns
import java.net.Inet6Address
import java.net.InetAddress
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
    }
}

/**
 * Parses one IPv4 dotted quad (0-255, no leading zeros) or IPv6 literal (no zone id), or returns
 * null. The bytes are built here and handed to [InetAddress.getByAddress], because `getByName` on
 * Android does a network lookup for anything it doesn't take for a literal (and reads `010` as octal).
 */
fun parseIpLiteral(text: String): InetAddress? {
    val bytes = parseIpv4(text) ?: parseIpv6(text) ?: return null
    return InetAddress.getByAddress(bytes)
}

private val ipv4Part = Regex("""0|[1-9]\d{0,2}""")

private fun parseIpv4(text: String): ByteArray? {
    val parts = text.split('.')
    if (parts.size != 4 || !parts.all { ipv4Part.matches(it) && it.toInt() <= 255 }) return null
    return ByteArray(4) { parts[it].toInt().toByte() }
}

private fun parseIpv6(text: String): ByteArray? {
    if (text.count { it == ':' } < 2) return null
    val gap = text.indexOf("::")
    if (gap >= 0 && text.indexOf("::", gap + 1) >= 0) return null
    val head = ipv6Words(if (gap >= 0) text.substring(0, gap) else text, mayEndInIpv4 = gap < 0) ?: return null
    val tail = if (gap >= 0) ipv6Words(text.substring(gap + 2), mayEndInIpv4 = true) ?: return null else emptyList()
    val given = head.size + tail.size
    if (if (gap >= 0) given > 7 else given != 8) return null
    val words = head + List(8 - given) { 0 } + tail
    return ByteArray(16) { i -> (words[i / 2] shr (if (i % 2 == 0) 8 else 0)).toByte() }
}

/** The 16-bit groups of one side of an IPv6 literal; a trailing dotted quad counts as two. */
private fun ipv6Words(part: String, mayEndInIpv4: Boolean): List<Int>? {
    if (part.isEmpty()) return emptyList()
    val groups = part.split(':')
    val words = ArrayList<Int>(8)
    for ((i, group) in groups.withIndex()) {
        if (mayEndInIpv4 && i == groups.lastIndex && '.' in group) {
            val quad = parseIpv4(group) ?: return null
            words += (quad[0].toInt() and 0xFF shl 8) or (quad[1].toInt() and 0xFF)
            words += (quad[2].toInt() and 0xFF shl 8) or (quad[3].toInt() and 0xFF)
        } else {
            if (group.length !in 1..4 || !group.all { it.isDigit() || it in 'a'..'f' || it in 'A'..'F' }) return null
            words += group.toInt(16)
        }
    }
    return words
}

/**
 * RFC 1918 / link-local, plus IPv6 unique-local (fc00::/7). Not loopback: that's what a DNS sinkhole
 * answers, and the server is never on the watch itself.
 */
internal fun InetAddress.isPrivate(): Boolean =
    isSiteLocalAddress || isLinkLocalAddress ||
        (this is Inet6Address && (address[0].toInt() and 0xFE) == 0xFC)
